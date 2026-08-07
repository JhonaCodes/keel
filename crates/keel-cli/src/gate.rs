// SPDX-License-Identifier: Apache-2.0
//! `keel gate` — the client-hook bridge (spec section 12.1, D-012 refinement).
//!
//! The parent runtime (PTY + shims + OS sandbox) governs the child's OS
//! ACTIONS, but it cannot see the client's INTERNAL tool calls (Claude Code's
//! Write/Edit). This bridge closes that gap: keel installs a PreToolUse hook
//! (from OUTSIDE, per session) that forwards the client's tool call here;
//! `keel gate` evaluates it with the SAME engine and returns the decision by
//! exit code — exit 2 blocks the tool BEFORE it runs (PreToolUse is
//! pre-action), with the ContextPacket on stderr.
//!
//! The hook is pure TRANSPORT — no rule logic lives in it. It is a COMPLEMENT:
//! the hard rings do not depend on it, and the OS sandbox is what makes it
//! un-removable (it denies the child writing the hook's own config). This is
//! not the old "client hook is the only, editable defense" — it is a protected
//! visibility bridge.

use anyhow::{Context, Result};
use keel_core::Decision;
use keel_core::event::{Event, EventKind};
use keel_engine::ledger::{Ledger, new_ev_id, now_ts};
use keel_engine::packet;
use keel_engine::runtime::{Mode, evaluate_event};
use keel_engine::snapshot::Snapshot;
use keel_engine::workspace::WorkspaceFiles;
use keel_runtime::RuntimeStore;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

const RUNTIME_DB: &str = "runtime.sqlite";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    /// stdin carries a Keel `Event` JSON directly (tests / other adapters).
    Native,
    /// stdin carries a Claude Code hook payload (PreToolUse/PostToolUse/Stop).
    ClaudeCode,
}

/// Reads one hook payload from stdin, evaluates it, and returns the exit code
/// the client must honor: 2 = block the action, 0 = allow.
pub fn gate(root: &Path, client: Client, session_flag: Option<String>) -> Result<ExitCode> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("gate: could not read the hook payload from stdin")?;

    // Unknown / ungoverned hook shapes pass through silently: the bridge only
    // governs what it understands; breaking the client on unrelated hooks would
    // make it fragile, not safe.
    let Some((mut event, preventable)) = parse(client, &input) else {
        return Ok(ExitCode::SUCCESS);
    };
    if event.session_id.is_none() {
        event.session_id = session_flag;
    }

    let files = WorkspaceFiles::empty(root.to_path_buf());
    let snapshot = match Snapshot::load(&files.snapshot_path()) {
        Ok(s) => s,
        // No published snapshot → nothing to enforce; never break the client.
        Err(_) => return Ok(ExitCode::SUCCESS),
    };

    // Session skills loaded through keel so far, so `skill.loaded` gates apply
    // to internal tool calls too (e.g. require a skill before a Write).
    let session_id = event
        .session_id
        .clone()
        .unwrap_or_else(|| "anonymous".into());
    event.loaded_skills = RuntimeStore::open(&files.state_dir().join(RUNTIME_DB))
        .ok()
        .and_then(|store| store.consumed_skill_ids(&session_id).ok())
        .unwrap_or_default();

    let ledger = Ledger::open(&files.ledger_path())?;
    let evals = evaluate_event(&snapshot, &event, &files.root, Mode::Enforce);

    let mut worst = Decision::Allow;
    let mut packets: Vec<String> = Vec::new();
    for eval in &evals {
        let entry = eval.to_ledger_entry(&event, &snapshot.hash.to_string(), new_ev_id(), now_ts());
        ledger.append(&entry)?;
        if eval.effective_decision >= Decision::Review {
            packets.push(packet::render(
                eval,
                &entry.id,
                &[],
                &snapshot.hash.to_string(),
            ));
        }
        worst = worst.max(eval.effective_decision);
    }

    for p in &packets {
        eprintln!("{p}\n");
    }

    // PreToolUse (and Stop) fire BEFORE the action → a block can truly prevent
    // it (exit 2). PostToolUse is post-hoc → feedback only (exit 0): the tool
    // already ran, so a false exit-2 would be a lie (invariant 8).
    if worst >= Decision::Block && preventable {
        Ok(ExitCode::from(2))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// Parses the payload into a keel event and whether a block on it can PREVENT
/// the action (pre-action hook) vs only give feedback (post-hoc).
fn parse(client: Client, input: &str) -> Option<(Event, bool)> {
    match client {
        Client::Native => serde_json::from_str::<Event>(input).ok().map(|e| (e, true)),
        Client::ClaudeCode => parse_claude_code_hook(input),
    }
}

/// Translates a Claude Code hook payload into a keel event + preventability
/// (spec section 12.1 adapter contract, deliberately thin). Unknown shapes →
/// None (pass-through).
///
/// - PreToolUse  + Bash                     → command.requested   (preventable)
/// - PreToolUse  + Edit/Write/MultiEdit     → file.edited         (preventable: not written yet)
/// - PostToolUse + Bash                     → command.completed   (feedback)
/// - PostToolUse + Edit/Write/MultiEdit     → file.edited         (feedback)
/// - Stop                                   → completion.requested (preventable, section 12.3)
fn parse_claude_code_hook(input: &str) -> Option<(Event, bool)> {
    let v: serde_json::Value = serde_json::from_str(input).ok()?;
    let hook = v.get("hook_event_name")?.as_str()?;
    let session_id = v
        .get("session_id")
        .and_then(|s| s.as_str())
        .map(str::to_string);

    let mk = |kind: EventKind| Event {
        kind,
        session_id: session_id.clone(),
        file: None,
        language: None,
        content: None,
        line: None,
        command: None,
        env: Default::default(),
        files: vec![],
        loaded_skills: vec![],
    };

    let pre = hook == "PreToolUse";
    match hook {
        "PreToolUse" | "PostToolUse" => {
            let tool = v.get("tool_name")?.as_str()?;
            let ti = v.get("tool_input")?;
            match tool {
                "Bash" => {
                    let kind = if pre {
                        EventKind::CommandRequested
                    } else {
                        EventKind::CommandCompleted
                    };
                    let mut ev = mk(kind);
                    ev.command = ti
                        .get("command")
                        .and_then(|c| c.as_str())
                        .map(str::to_string);
                    Some((ev, pre))
                }
                "Edit" | "Write" | "MultiEdit" => {
                    let mut ev = mk(EventKind::FileEdited);
                    ev.file = ti
                        .get("file_path")
                        .and_then(|f| f.as_str())
                        .map(str::to_string);
                    ev.content = ti
                        .get("content")
                        .and_then(|c| c.as_str())
                        .map(str::to_string)
                        .or_else(|| {
                            ti.get("new_string")
                                .and_then(|c| c.as_str())
                                .map(str::to_string)
                        })
                        .or_else(|| {
                            let edits = ti.get("edits")?.as_array()?;
                            let joined: Vec<&str> = edits
                                .iter()
                                .filter_map(|e| e.get("new_string")?.as_str())
                                .collect();
                            Some(joined.join("\n"))
                        });
                    // PreToolUse Write has not landed yet → preventable.
                    Some((ev, pre))
                }
                _ => None,
            }
        }
        "Stop" => Some((mk(EventKind::CompletionRequested), true)),
        _ => None,
    }
}
