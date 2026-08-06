// SPDX-License-Identifier: Apache-2.0
//! `keel gate` — the pre-action entry point. This is where the keel holds.
//!
//! One event in (stdin) → evaluate in `Enforce` mode → ledger → decide:
//!
//! | Worst effective decision | Exit | Output |
//! |---|---|---|
//! | allow                    | 0    | silent |
//! | review                   | 0    | advisory packet on stderr (section 4.7: reversible — CI catches it) |
//! | block / deny-pending-approval | **2** | ContextPacket on stderr — the client MUST NOT run the action |
//!
//! Inner ring (spec section 5.3): for `command.requested` the client asks BEFORE
//! executing, so a blocked command **never exists as a process**. The exit-2
//! contract is the same one agent clients use for hooks: blocking is not
//! advice, it prevents the action, and the packet contextualizes WHY plus how
//! to correct (section 6.5: a blocking packet must reduce ambiguity to near zero).
//!
//! `--client claude-code` adapts the Claude Code hook protocol: the hook is
//! pure transport (section 12.2) — it forwards its stdin here and applies our
//! decision. The rule logic never lives in the hook.

use anyhow::{Context, Result};
use keel_core::event::{Event, EventKind};
use keel_core::Decision;
use keel_engine::ledger::Ledger;
use keel_engine::packet;
use keel_engine::runtime::{evaluate_event, Mode};
use keel_engine::session::{deliver_skills, SessionStore};
use keel_engine::snapshot::Snapshot;
use keel_engine::workspace;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::{new_ev_id, now_ts, to_ledger_entry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    /// stdin carries a Keel `Event` JSON directly.
    Native,
    /// stdin carries a Claude Code hook payload (PreToolUse/PostToolUse/Stop).
    ClaudeCode,
}

pub fn gate(
    root: &Path,
    client: Client,
    session_flag: Option<String>,
    passive: bool,
) -> Result<ExitCode> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("gate: could not read the event from stdin")?;

    let mut event = match parse_event(client, &input)? {
        Some(ev) => ev,
        // Unknown hook kinds pass through silently: the gate only governs
        // what it understands; breaking the client on unrelated hooks would
        // make the adapter fragile, not safe.
        None => return Ok(ExitCode::SUCCESS),
    };

    // ADR-022: preconditions evaluate the state of the world AT REQUEST TIME.
    // At the gate we are live (not replaying), so capture the real process
    // environment — event-provided values win over inherited ones.
    for (k, v) in std::env::vars() {
        event.env.entry(k).or_insert(v);
    }
    if event.session_id.is_none() {
        event.session_id = session_flag;
    }

    let files = workspace::load(root)?;
    let snapshot = Snapshot::load(&files.snapshot_path())
        .context("no published snapshot — run `keel compile` first")?;
    std::fs::create_dir_all(files.state_dir())?;
    let ledger = Ledger::open(&files.ledger_path())?;

    // `--passive` = shadow mode: evaluate and record, never block. For trying
    // new rules without risk; the ledger still captures declared decisions.
    let mode = if passive { Mode::Passive } else { Mode::Enforce };
    let evals = evaluate_event(&snapshot, &event, &files.root, mode);

    // L2 session state: which skills this session already has in context.
    let store = SessionStore::new(&files.state_dir());
    let session_id = event.session_id.clone().unwrap_or_else(|| "anonymous".into());
    let mut session_state = store.load(&session_id);

    let mut worst = Decision::Allow;
    let mut packets: Vec<String> = Vec::new();

    for eval in &evals {
        let entry = to_ledger_entry(eval, &event, &snapshot, new_ev_id(), now_ts());
        ledger.append(&entry)?;

        // Oscillation guard (section 6.5): a runtime that blocks is responsible for
        // not inducing token-burn loops. Same rule+location+session repeating
        // ≥3 times → escalate explicitly AND deliver the full skill variant.
        let oscillating = is_oscillating(&ledger, &entry.rule_id, &event);

        // L2 cognitive activation: deliver skill content once per session,
        // escalate to full on oscillation, reference-only when loaded.
        let mut payload = deliver_skills(
            &snapshot,
            &files.root,
            &mut session_state,
            &eval.load_skills,
            oscillating,
        );
        if oscillating {
            payload.push(
                "oscillating: repeated finding at this location — stop retrying; \
                 request human intervention if it persists (section 6.5)"
                    .to_string(),
            );
        }

        // A packet is emitted when something needs the model's attention:
        // any decision above allow, or a cognitive activation with payload.
        if eval.effective_decision >= Decision::Review || !payload.is_empty() {
            packets.push(packet::render(eval, &entry.id, &payload));
        }
        worst = worst.max(eval.effective_decision);
    }

    store.save(&session_id, &session_state, &now_ts())?;

    // Whether a block on THIS event can actually be prevented (section 5.3):
    // - inner ring (command/transition/delivery.requested): pre-action — the
    //   client asks before executing, so exit 2 stops the action.
    // - completion.requested: the Stop transition is gate-able (section 12.3).
    // - outer ring (file.edited, *.completed): post-hoc — the file already
    //   landed. A block here is FEEDBACK, not prevention (exit 0): the file is
    //   reversible and inert; its danger only materializes at execution, which
    //   crosses the inner ring. Emitting exit 2 would be a false promise.
    let preventable =
        event.kind.is_inner_ring() || event.kind == EventKind::CompletionRequested;

    // Completion gate (section 12.3, section 6.2 minimal): "done" is not a claim the model
    // makes — it is a transition the runtime authorizes. Live blockers
    // (invalid findings never followed by a later `valid` of the same
    // rule+file in this session) veto the close, with the pending list.
    if event.kind == EventKind::CompletionRequested && !passive {
        let blockers = ledger.unresolved_blockers(&session_id)?;
        if !blockers.is_empty() {
            let mut lines = vec![format!(
                "COMPLETION DENIED — {} unresolved blocker(s) in session {session_id} (section 12.3)",
                blockers.len()
            )];
            for b in &blockers {
                let loc = match (&b.file, b.line) {
                    (Some(f), Some(l)) => format!(" at {f}:{l}"),
                    (Some(f), None) => format!(" at {f}"),
                    _ => String::new(),
                };
                lines.push(format!(
                    "  {}{loc} — {} (evidence {})",
                    b.rule_id,
                    b.detail.as_deref().unwrap_or("blocking finding"),
                    b.id
                ));
            }
            lines.push("resolve each finding (a later valid evaluation clears it) before closing".into());
            packets.push(lines.join("\n"));
            worst = worst.max(Decision::Block);
        }
    }

    for p in &packets {
        eprintln!("{p}\n");
    }

    // The exit code IS the contract: 2 prevents the action, 0 allows it. Only
    // preventable events (section 5.3 inner ring + completion) exit 2; a post-hoc
    // block is delivered as feedback (packet on stderr) with exit 0 — the edit
    // already happened and the danger is caught later at execution.
    if worst >= Decision::Block && preventable {
        Ok(ExitCode::from(2))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn is_oscillating(ledger: &Ledger, rule_id: &str, event: &Event) -> bool {
    ledger
        .oscillations(3)
        .unwrap_or_default()
        .iter()
        .any(|o| {
            o.rule_id == rule_id
                && o.session_id == event.session_id
                && o.file == event.file
                && o.line == event.line
        })
}

fn parse_event(client: Client, input: &str) -> Result<Option<Event>> {
    match client {
        Client::Native => {
            let ev: Event = serde_json::from_str(input)
                .context("gate: stdin is not a valid Keel event")?;
            Ok(Some(ev))
        }
        Client::ClaudeCode => Ok(parse_claude_code_hook(input)),
    }
}

/// Translates a Claude Code hook payload into a Keel event (section 12.1 adapter
/// contract, kept deliberately thin). Unknown shapes → None (pass-through).
///
/// Mapping:
/// - PreToolUse  + Bash            → command.requested   (inner ring: pre-action)
/// - PostToolUse + Bash            → command.completed
/// - Pre/PostToolUse + Edit/Write/MultiEdit → file.edited (outer ring section 5.3:
///   post-hoc is acceptable — an edited file is reversible and inert; its
///   danger materializes at execution/delivery, which cross the inner ring)
/// - Stop                          → completion.requested (section 12.3)
fn parse_claude_code_hook(input: &str) -> Option<Event> {
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
    };

    match hook {
        "PreToolUse" | "PostToolUse" => {
            let tool = v.get("tool_name")?.as_str()?;
            let ti = v.get("tool_input")?;
            match tool {
                "Bash" => {
                    let kind = if hook == "PreToolUse" {
                        EventKind::CommandRequested
                    } else {
                        EventKind::CommandCompleted
                    };
                    let mut ev = mk(kind);
                    ev.command = ti.get("command").and_then(|c| c.as_str()).map(str::to_string);
                    Some(ev)
                }
                "Edit" | "Write" | "MultiEdit" => {
                    let mut ev = mk(EventKind::FileEdited);
                    ev.file = ti
                        .get("file_path")
                        .and_then(|f| f.as_str())
                        .map(str::to_string);
                    // The content under governance is what lands in the file:
                    // Write → full content; Edit → the new_string; MultiEdit →
                    // all new_strings concatenated.
                    ev.content = ti
                        .get("content")
                        .and_then(|c| c.as_str())
                        .map(str::to_string)
                        .or_else(|| {
                            ti.get("new_string").and_then(|c| c.as_str()).map(str::to_string)
                        })
                        .or_else(|| {
                            let edits = ti.get("edits")?.as_array()?;
                            let joined: Vec<&str> = edits
                                .iter()
                                .filter_map(|e| e.get("new_string")?.as_str())
                                .collect();
                            Some(joined.join("\n"))
                        });
                    Some(ev)
                }
                _ => None, // other tools are not governed in this slice
            }
        }
        "Stop" => Some(mk(EventKind::CompletionRequested)),
        _ => None,
    }
}

/// `keel adapter claude-code --print` — emits the settings.json wiring. The
/// hook only transports events and applies our exit code; no rule logic
/// lives in it (section 12.2, ADR-003).
pub fn adapter_print_claude_code(root: &Path) -> Result<ExitCode> {
    let ws = root
        .canonicalize()
        .unwrap_or_else(|_| root.to_path_buf());
    let cmd = format!("keel gate --client claude-code --workspace {}", ws.display());
    let snippet = serde_json::json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "Bash|Edit|Write|MultiEdit",
                "hooks": [{ "type": "command", "command": cmd }]
            }],
            "PostToolUse": [{
                "matcher": "Edit|Write|MultiEdit",
                "hooks": [{ "type": "command", "command": cmd }]
            }],
            "Stop": [{
                "hooks": [{ "type": "command", "command": cmd }]
            }]
        }
    });
    println!("{}", serde_json::to_string_pretty(&snippet)?);
    println!();
    println!("# Add the block above to your project's .claude/settings.json.");
    println!("# Contract: exit 2 blocks the action (the packet on stderr reaches the model);");
    println!("# exit 0 allows. Verify the hook schema against your client version (spec section 19).");
    Ok(ExitCode::SUCCESS)
}

/// `keel audit --agent <id> --input <file>` — invokes a specialized agent
/// (spec section 14) on the given material. The result is recorded with
/// origin=semantic (section 6.4) and is advisory (review), never a block (section 4.7).
pub fn audit(root: &Path, agent_id: &str, input: &Path, session: Option<String>) -> Result<ExitCode> {
    use keel_engine::audit::{run_audit, AgentSpec, ExecutorSpec};

    let files = workspace::load(root)?;
    let snapshot = Snapshot::load(&files.snapshot_path())
        .context("no published snapshot — run `keel compile` first")?;
    std::fs::create_dir_all(files.state_dir())?;
    let ledger = Ledger::open(&files.ledger_path())?;

    let agent = files
        .agents
        .iter()
        .find(|a| a.metadata.id == agent_id)
        .with_context(|| format!("no agent `{agent_id}` in agents/"))?;
    let executor_id = agent
        .spec
        .executor
        .strip_prefix("executor:")
        .unwrap_or(&agent.spec.executor);
    let executor = files
        .executors
        .iter()
        .find(|x| x.metadata.id == executor_id)
        .with_context(|| format!("agent `{agent_id}` routes to executor `{executor_id}`, not found in agents/"))?;

    let material = std::fs::read_to_string(input)
        .with_context(|| format!("could not read material {}", input.display()))?;

    let agent_spec = AgentSpec {
        id: agent.metadata.id.clone(),
        role: agent.spec.role.clone(),
        objective: agent.spec.objective.clone(),
        timeout_ms: agent.spec.budget.as_ref().and_then(|b| b.timeout_ms).unwrap_or(60_000),
    };
    let exec_spec = ExecutorSpec {
        id: executor.metadata.id.clone(),
        command: executor.spec.command.clone(),
        model: executor.spec.model.clone(),
        timeout_ms: executor.spec.timeout_ms,
    };

    let out = run_audit(
        &agent_spec, &exec_spec, &material, &snapshot.hash.to_string(),
        session.as_deref(), &ledger, new_ev_id(), now_ts(),
    );

    println!("agent    {} (role: {})", agent_spec.id, agent_spec.role);
    println!("executor {}", exec_spec.id);
    println!("verdict  {} (origin: semantic — a model's opinion, advisory only section 4.7)",
        serde_json::to_string(&out.verdict).unwrap_or_default().trim_matches('"'));
    for f in &out.findings {
        println!("  - {f}");
    }
    println!("evidence {}", out.evidence_id);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
#[path = "../tests-unit/gate.rs"]
mod tests;