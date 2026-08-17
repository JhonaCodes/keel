// SPDX-License-Identifier: Apache-2.0
//! The pre-action broker — where the keel holds in the parent runtime.
//!
//! One shim request in (UNIX socket, JSON) → `command.requested` evaluated in
//! `Enforce` mode → ledger → decision out. The exit contract the shim applies
//! is the inner-ring contract (spec section 5.3): a blocked command NEVER
//! exists as a process, and the ContextPacket the child sees on stderr
//! contextualizes WHY plus how to correct (section 6.5).
//!
//! Wire protocol (one JSON object per line, one request per connection):
//!   → {"name":"rm","argv":["rm","notes.md"],"cwd":"/work"}
//!   ← {"allow":false,"packet":"BLOCKED (…)…"}
//!
//! The broker is the ONLY writer of the ledger during a session, and it holds
//! the snapshot in memory: rules are pinned for the whole session (the
//! artifact, not the workspace files, is the authority — invariant 4).

use anyhow::{Context, Result};
use keel_core::Decision;
use keel_core::audit::target_for_command;
use keel_core::event::{Event, EventKind};
use keel_engine::ledger::{Ledger, new_ev_id, now_ts};
use keel_engine::packet;
use keel_engine::runtime::{Mode, evaluate_event};
use keel_engine::snapshot::Snapshot;
use keel_runtime::RuntimeStore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Serialize, Deserialize)]
pub struct ShimRequest {
    pub name: String,
    pub argv: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ShimResponse {
    pub allow: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packet: Option<String>,
}

pub struct Broker {
    snapshot: Snapshot,
    ledger: Ledger,
    root: PathBuf,
    session_id: String,
    /// Environment captured ONCE at session start (ADR-022: preconditions see
    /// the state of the world; capturing at start keeps every evaluation of
    /// the session against the same world).
    env: BTreeMap<String, String>,
}

impl Broker {
    pub fn new(snapshot: Snapshot, ledger: Ledger, root: PathBuf, session_id: String) -> Self {
        Broker {
            snapshot,
            ledger,
            root,
            session_id,
            env: std::env::vars().collect(),
        }
    }

    /// Skills this session has loaded through keel so far, read live from the
    /// runtime store (a skill loaded via `keel.skills.load` a moment ago must
    /// count now). Best-effort: on any store error, empty — which makes a
    /// `skill.loaded` gate fail closed (block + tell the model to load it).
    fn loaded_skills(&self) -> Vec<String> {
        let db = self.root.join(".keel-state").join("runtime.sqlite");
        RuntimeStore::open(&db)
            .ok()
            .and_then(|store| store.consumed_skill_ids(&self.session_id).ok())
            .unwrap_or_default()
    }

    /// Distinct (event_kind, verdict, rule_id) triples already recorded in THIS
    /// session's ledger, read live before evaluation — same pattern as
    /// `loaded_skills` but generic to any event kind. Best-effort: on any
    /// error, empty, which makes `evidence.recorded` fail closed (block).
    fn recorded_evidence(&self) -> Vec<(keel_core::event::EventKind, keel_core::Verdict, String)> {
        self.ledger
            .recorded_evidence(&self.session_id)
            .unwrap_or_default()
    }

    /// This session's audits, plus every GO recorded for `scope` whatever
    /// session observed it: the scope is the hash of the patch, so it identifies
    /// the change-set, not the observer. See `Ledger::audits_for_scope`.
    fn recorded_audits(&self, scope: Option<&str>) -> Vec<keel_core::event::AuditEvidence> {
        let mut audits = self
            .ledger
            .recorded_audits(&self.session_id)
            .unwrap_or_default();
        if let Some(scope) = scope
            && let Ok(mut for_scope) = self.ledger.audits_for_scope(scope)
        {
            audits.append(&mut for_scope);
        }
        audits
    }

    /// Evaluates one interposed command. Pure with respect to the socket —
    /// used directly by unit tests.
    pub fn decide(&self, req: &ShimRequest) -> Result<ShimResponse> {
        let command = req.argv.join(" ");
        // The shim reports the directory the command was launched from, and that
        // is the repo whose change-set is at stake. `self.root` is the workspace
        // — the rules live there, not the code being shipped — so it is only the
        // fallback. Same defect the hook bridge had.
        let audit_root =
            keel_core::audit::repo_root(req.cwd.as_deref()).unwrap_or_else(|| self.root.clone());
        let target = target_for_command(&audit_root, Some(&command));
        let event = Event {
            kind: EventKind::CommandRequested,
            session_id: Some(self.session_id.clone()),
            file: None,
            language: None,
            content: None,
            line: None,
            command: Some(command),
            env: self.env.clone(),
            files: Vec::new(),
            loaded_skills: self.loaded_skills(),
            recorded_evidence: self.recorded_evidence(),
            audit_scope: target.as_ref().map(|target| target.scope.clone()),
            audit_mode: target.as_ref().map(|target| target.mode.clone()),
            recorded_audits: self.recorded_audits(target.as_ref().map(|t| t.scope.as_str())),
        };

        let evals = evaluate_event(&self.snapshot, &event, &self.root, Mode::Enforce);

        let mut worst = Decision::Allow;
        let mut packets: Vec<String> = Vec::new();
        for eval in &evals {
            let entry = eval.to_ledger_entry(
                &event,
                &self.snapshot.hash.to_string(),
                new_ev_id(),
                now_ts(),
            );
            self.ledger.append(&entry)?;
            if eval.effective_decision >= Decision::Review {
                packets.push(packet::render(
                    eval,
                    &entry.id,
                    &[],
                    &self.snapshot.hash.to_string(),
                ));
            }
            worst = worst.max(eval.effective_decision);
        }

        Ok(ShimResponse {
            // command.requested is inner ring: prevention is real (section 5.3).
            allow: worst < Decision::Block,
            packet: if packets.is_empty() {
                None
            } else {
                Some(packets.join("\n\n"))
            },
        })
    }

    /// Serves shim requests until `shutdown` flips. One request per
    /// connection, handled serially: the shell serializes commands anyway,
    /// and a single thread keeps the ledger single-writer by construction.
    pub fn serve(&self, listener: UnixListener, shutdown: Arc<AtomicBool>) {
        for stream in listener.incoming() {
            if shutdown.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else { continue };
            if let Err(e) = self.handle(stream) {
                // A malformed shim request must never kill the session; the
                // failure is visible and the NEXT command still gets policy.
                eprintln!("[keel] broker: request failed: {e:#}");
            }
        }
    }

    fn handle(&self, stream: UnixStream) -> Result<()> {
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("broker: could not read the shim request")?;
        let req: ShimRequest =
            serde_json::from_str(&line).context("broker: shim request is not valid JSON")?;
        let resp = self.decide(&req)?;
        let mut stream = stream;
        let payload = serde_json::to_string(&resp)?;
        stream.write_all(payload.as_bytes())?;
        stream.write_all(b"\n")?;
        Ok(())
    }
}

/// Binds the session socket (0700 parent dir is the caller's job) and spawns
/// the serve loop. Returns the shutdown handle; flipping it and touching the
/// socket unblocks `accept`.
pub fn spawn(
    broker: Broker,
    socket_path: &Path,
) -> Result<(std::thread::JoinHandle<()>, BrokerHandle)> {
    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("broker: could not bind {}", socket_path.display()))?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let handle = BrokerHandle {
        shutdown: shutdown.clone(),
        socket_path: socket_path.to_path_buf(),
    };
    let join = std::thread::spawn(move || broker.serve(listener, shutdown));
    Ok((join, handle))
}

pub struct BrokerHandle {
    shutdown: Arc<AtomicBool>,
    socket_path: PathBuf,
}

impl BrokerHandle {
    /// Stops the serve loop: flips the flag and pokes the socket so a blocked
    /// `accept` wakes up and observes it.
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.socket_path);
    }
}

#[cfg(test)]
#[path = "../tests-unit/broker.rs"]
mod tests;
