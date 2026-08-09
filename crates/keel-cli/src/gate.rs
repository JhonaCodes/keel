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
    /// stdin carries a Codex hook payload (PreToolUse/PostToolUse/Stop).
    Codex,
}

/// Reads one hook payload from stdin, evaluates it, and returns the exit code
/// the client must honor: 2 = block the action, 0 = allow.
pub fn gate(root: &Path, client: Client, session_flag: Option<String>) -> Result<ExitCode> {
    // Load `<workspace>/.env` so `${VAR}` in governed configs resolves here too
    // (standalone-safe; a shell export still wins).
    keel_host::dotenv::load_workspace_env(root);

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

    // Evidence is best-effort: a ledger that cannot be written (e.g. a
    // read-only sandbox) must NEVER turn a block into a pass. The decision
    // comes from the engine, not from a successful ledger write. Opened
    // BEFORE evaluation (not just for the append loop below) so
    // `evidence.recorded` preconditions can see this session's history too.
    let ledger = Ledger::open(&files.ledger_path()).ok();
    event.recorded_evidence = ledger
        .as_ref()
        .and_then(|l| l.recorded_evidence(&session_id).ok())
        .unwrap_or_default();
    let evals = evaluate_event(&snapshot, &event, &files.root, Mode::Enforce);

    let mut worst = Decision::Allow;
    let mut packets: Vec<String> = Vec::new();
    for eval in &evals {
        let entry = eval.to_ledger_entry(&event, &snapshot.hash.to_string(), new_ev_id(), now_ts());
        if let Some(ledger) = &ledger
            && let Err(e) = ledger.append(&entry)
        {
            eprintln!("[keel] gate: evidence not recorded: {e}");
        }
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

    // Prompt enrichment (D-013): on the operator's prompt, keel DELIVERS the
    // tool output of every `prompt.submitted` rule to the model as context — the
    // model receives the task already deserialized (e.g. a ticket decomposed
    // with its id) instead of fetching it itself. Deterministic, by code, not a
    // model choice. Emitted as the client's `additionalContext`; never blocks.
    if event.kind == EventKind::PromptSubmitted {
        return Ok(emit_delivery(
            &evals,
            &snapshot,
            event.content.as_deref(),
            &files.root,
            "UserPromptSubmit",
            "contexto entregado al modelo",
        ));
    }
    // Session start (D-016): deliver the base context/rules up front — no moment
    // text, so only `session.started`-scoped rules and their skills surface.
    if event.kind == EventKind::SessionStarted {
        return Ok(emit_delivery(
            &evals,
            &snapshot,
            None,
            &files.root,
            "SessionStart",
            "contexto de inicio",
        ));
    }

    for p in &packets {
        eprintln!("{p}\n");
    }

    // PreToolUse (and Stop) fire BEFORE the action → a block can truly prevent
    // it (exit 2). PostToolUse is post-hoc → feedback only (exit 0): the tool
    // already ran, so a false exit-2 would be a lie (invariant 8).
    if worst >= Decision::Block && preventable {
        return Ok(ExitCode::from(2));
    }

    // Opportune delivery (D-016): a pre-action tool moment that did NOT block is
    // the moment to hand the model the relevant rules/skills/agents/blueprints —
    // "you're about to touch this, keel has these". Uses the file content / the
    // command as the moment text; emits `additionalContext` without a permission
    // decision, so the client's own approval flow is untouched.
    if preventable
        && matches!(
            event.kind,
            EventKind::FileEdited | EventKind::CommandRequested | EventKind::ToolRequested
        )
    {
        let moment = event
            .content
            .as_deref()
            .or(event.command.as_deref())
            .or(event.file.as_deref());
        return Ok(emit_delivery(
            &evals,
            &snapshot,
            moment,
            &files.root,
            "PreToolUse",
            "recursos en este momento",
        ));
    }

    Ok(ExitCode::SUCCESS)
}

/// Assembles the context keel delivers at a MOMENT (D-013/D-014/D-016):
///
/// 1. Rule enrichment (D-013): a matched rule's tool output — the task already
///    deserialized (a decomposed ticket, a PR).
/// 2. Routed catalog (D-014/D-016): the ranked, relevant skills / agents /
///    governed components (blueprints, knowledge…) for `moment` — "you're about
///    to touch this, keel has these" — plus any skill a matched rule asked to
///    load (`enforcement.*.load.skills`). An `autoload` skill is injected whole.
///
/// Returns the joined context and the sources (for the operator banner), or
/// `None` when nothing is relevant — silence, never noise (a trivial `Read`
/// surfaces nothing).
fn build_delivery_context(
    evals: &[keel_engine::runtime::Evaluation],
    snapshot: &Snapshot,
    moment: Option<&str>,
    root: &Path,
) -> Option<(String, Vec<String>)> {
    let mut blocks: Vec<String> = Vec::new();
    let mut sources: Vec<String> = Vec::new();

    // 1) Rule enrichment (D-013): structured tool output (e.g. a decomposed ticket).
    for eval in evals {
        let before = blocks.len();
        for finding in &eval.findings {
            if !finding.message.trim().is_empty() {
                blocks.push(finding.message.clone());
            }
        }
        if blocks.len() > before {
            sources.push(eval.rule_id.clone());
        }
    }

    // 2) Skills a matched rule attached (always — they don't need a moment to
    //    route on) + the routed catalog (only when there is moment text).
    let rule_skills: Vec<String> = evals
        .iter()
        .flat_map(|e| e.load_skills.iter())
        .map(|s| {
            let id = s.strip_prefix("skill:").unwrap_or(s.as_str());
            id.split('#').next().unwrap_or(id).to_string()
        })
        .collect();
    let routed = match moment {
        Some(moment) => keel_engine::routing::route(snapshot, moment, 6),
        None => keel_engine::routing::RouteResult::default(),
    };
    if let Some(block) = routed_catalog_block(snapshot, &routed, &rule_skills, root) {
        blocks.push(block);
        for s in &routed.skills {
            sources.push(format!("{}·{}", s.id, s.trigger));
        }
        for id in &rule_skills {
            sources.push(format!("{id}·rule"));
        }
    }

    if blocks.is_empty() {
        None
    } else {
        Some((blocks.join("\n\n"), sources))
    }
}

/// Emits the assembled context to the model through a hook's `additionalContext`
/// channel, tagged with `hook_event_name` (`UserPromptSubmit` on a prompt,
/// `PreToolUse` at a tool moment, `SessionStart` at session start). Never blocks
/// and — crucially at a tool moment — emits NO `permissionDecision`: keel adds
/// context but leaves the client's own approval flow untouched (forcing "allow"
/// would silently bypass the user's permission gate). Nothing relevant → no
/// output, so trivial moments stay quiet. On the prompt it also surfaces the
/// banner to the operator's screen (`systemMessage`).
fn emit_delivery(
    evals: &[keel_engine::runtime::Evaluation],
    snapshot: &Snapshot,
    moment: Option<&str>,
    root: &Path,
    hook_event_name: &str,
    banner_label: &str,
) -> ExitCode {
    let Some((context, sources)) = build_delivery_context(evals, snapshot, moment, root) else {
        return ExitCode::SUCCESS;
    };
    let banner = format!("keel ✦ {banner_label}: {}", sources.join(", "));
    eprintln!("[{banner}]");
    let mut out = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": hook_event_name,
            "additionalContext": context,
        }
    });
    if hook_event_name == "UserPromptSubmit" {
        out["systemMessage"] = serde_json::Value::String(banner);
    }
    println!("{out}");
    ExitCode::SUCCESS
}

/// Renders the routed capabilities into one context block: a token-thrifty
/// catalog (id + description + trigger) the model can pull from — skills, agents
/// and other governed components (blueprints, knowledge…) — plus the skills a
/// matched rule attached, and the full content of any `autoload` skill. `None`
/// when there is nothing to surface.
fn routed_catalog_block(
    snapshot: &Snapshot,
    routed: &keel_engine::routing::RouteResult,
    rule_skills: &[String],
    root: &Path,
) -> Option<String> {
    if routed.is_empty() && rule_skills.is_empty() {
        return None;
    }
    let mut lines = vec![
        "Keel tiene recursos gobernados relevantes a este momento (no hace falta que los busques):"
            .to_string(),
    ];

    // Skills: routed + those a matched rule attached (deduped, rule-attached first).
    let mut seen = std::collections::BTreeSet::new();
    let mut skill_lines = Vec::new();
    for id in rule_skills {
        if seen.insert(id.clone()) {
            skill_lines.push(catalog_skill_line(snapshot, id, "rule"));
        }
    }
    for s in &routed.skills {
        if seen.insert(s.id.clone()) {
            skill_lines.push(catalog_skill_line(snapshot, &s.id, &s.trigger));
        }
    }
    if !skill_lines.is_empty() {
        lines.push("\nSkills (cargá con keel.skills.load <id>):".to_string());
        lines.extend(skill_lines);
    }

    if !routed.agents.is_empty() {
        lines.push("\nAgentes (invocá con keel.agent.invoke agent=<id>):".to_string());
        for a in &routed.agents {
            let obj = snapshot
                .agents
                .get(&a.id)
                .and_then(|c| c.objective.as_deref())
                .unwrap_or("");
            let sep = if obj.is_empty() { "" } else { ": " };
            lines.push(format!("- {} [{}]{sep}{obj}", a.id, a.trigger));
        }
    }

    if !routed.components.is_empty() {
        lines.push("\nBlueprints/knowledge (consultá con keel.blueprints):".to_string());
        for c in &routed.components {
            let desc = snapshot
                .components
                .get(&c.id)
                .and_then(|cc| cc.description.as_deref())
                .unwrap_or("");
            let sep = if desc.is_empty() { "" } else { ": " };
            lines.push(format!("- {} [{}]{sep}{desc}", c.id, c.trigger));
        }
    }

    // Autoload: inject the compact content of strongly-matched opt-in skills.
    for s in routed.skills.iter().filter(|s| s.autoload) {
        if let Some(compact) = snapshot.skills.get(&s.id).map(|c| &c.compact)
            && let Ok(content) = std::fs::read_to_string(root.join(compact))
        {
            lines.push(format!("\n── skill cargada: {} ──\n{content}", s.id));
        }
    }

    lines.push(
        "\nEl listado completo está disponible con keel.skills.list en cualquier momento."
            .to_string(),
    );
    Some(lines.join("\n"))
}

/// One catalog line for a skill: `- <id> [<trigger>]: <description>`.
fn catalog_skill_line(snapshot: &Snapshot, id: &str, trigger: &str) -> String {
    let desc = snapshot
        .skills
        .get(id)
        .and_then(|c| c.description.as_deref())
        .unwrap_or("");
    let sep = if desc.is_empty() { "" } else { ": " };
    format!("- {id} [{trigger}]{sep}{desc}")
}

/// Parses the payload into a keel event and whether a block on it can PREVENT
/// the action (pre-action hook) vs only give feedback (post-hoc).
fn parse(client: Client, input: &str) -> Option<(Event, bool)> {
    match client {
        Client::Native => serde_json::from_str::<Event>(input).ok().map(|e| (e, true)),
        Client::ClaudeCode => parse_claude_code_hook(input),
        Client::Codex => parse_codex_hook(input),
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
        recorded_evidence: vec![], // populated by gate() before evaluate_event
    };

    let pre = hook == "PreToolUse";
    match hook {
        "PreToolUse" | "PostToolUse" => {
            let tool = v.get("tool_name")?.as_str()?;
            let ti = v.get("tool_input")?;
            match tool {
                "Bash" => {
                    let command = ti
                        .get("command")
                        .and_then(|c| c.as_str())
                        .map(str::to_string);
                    // PostToolUse of a TEST-RUNNER command becomes durable
                    // RED/GREEN evidence: a `test.completed` event whose content
                    // carries the pass/fail signal (from the real exit code) so
                    // an authored rule classifies it and the ledger records it —
                    // the port of jflow's evidence-capture. Nothing is decided
                    // here; the event just carries the observed truth.
                    if !pre && command.as_deref().is_some_and(is_test_runner) {
                        let mut ev = mk(EventKind::TestCompleted);
                        ev.content = Some(test_outcome_content(&v));
                        ev.command = command;
                        return Some((ev, false));
                    }
                    let kind = if pre {
                        EventKind::CommandRequested
                    } else {
                        EventKind::CommandCompleted
                    };
                    let mut ev = mk(kind);
                    ev.command = command;
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
                "WebFetch" => {
                    // Fetching a URL is a requested, preventable action. keel
                    // surfaces the URL as `command` so a rule can gate on it —
                    // e.g. force a governed tool instead of reading a Linear /
                    // Jira / GitHub URL directly. Only PreToolUse can prevent
                    // it; a completed fetch is post-hoc feedback.
                    let kind = if pre {
                        EventKind::CommandRequested
                    } else {
                        EventKind::CommandCompleted
                    };
                    let mut ev = mk(kind);
                    ev.command = ti.get("url").and_then(|u| u.as_str()).map(str::to_string);
                    Some((ev, pre))
                }
                other => {
                    // Any other tool the model is about to use (a native MCP tool,
                    // a read, a search): keel SEES it (matcher is catch-all) and can
                    // DELIVER relevant context at that moment (D-016). Observe-only
                    // — never blocks unless an authored rule decides otherwise. A
                    // PostToolUse of an unmapped tool is ignored in Phase 1.
                    if !pre {
                        return None;
                    }
                    let mut ev = mk(EventKind::ToolRequested);
                    ev.command = Some(other.to_string());
                    // Tool name + a truncated view of its input as the moment text,
                    // so routing can match (a native `linear` call surfaces the
                    // linear resources).
                    let input = ti.to_string();
                    let input: String = input.chars().take(500).collect();
                    ev.content = Some(format!("{other} {input}"));
                    Some((ev, true))
                }
            }
        }
        "Stop" => Some((mk(EventKind::CompletionRequested), true)),
        "UserPromptSubmit" => {
            // The operator's prompt, BEFORE the model reasons. keel evaluates
            // `prompt.submitted` rules and DELIVERS their tool output / skills
            // to the model as context (enrichment), so the model receives the
            // task already deserialized instead of fetching it itself. Not a
            // block — the prompt proceeds, enriched.
            let mut ev = mk(EventKind::PromptSubmitted);
            ev.content = v.get("prompt").and_then(|p| p.as_str()).map(str::to_string);
            Some((ev, false))
        }
        "SessionStart" => {
            // Session start (startup / resume): a moment to deliver base context
            // and `session.started` rules up front (D-016). Delivery only, never
            // a block.
            Some((mk(EventKind::SessionStarted), false))
        }
        _ => None,
    }
}

/// Translates a Codex hook payload into a keel event + preventability.
/// Codex's hook protocol is Claude-style, but canonical edit hooks arrive as
/// `tool_name: "apply_patch"` with the patch in `tool_input.command`.
fn parse_codex_hook(input: &str) -> Option<(Event, bool)> {
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
        recorded_evidence: vec![],
    };

    let pre = hook == "PreToolUse";
    match hook {
        "PreToolUse" | "PostToolUse" => {
            let tool = v.get("tool_name")?.as_str()?;
            let ti = v.get("tool_input")?;
            match tool {
                "Bash" => {
                    let command = ti
                        .get("command")
                        .and_then(|c| c.as_str())
                        .map(str::to_string);
                    if !pre && command.as_deref().is_some_and(is_test_runner) {
                        let mut ev = mk(EventKind::TestCompleted);
                        ev.content = Some(test_outcome_content(&v));
                        ev.command = command;
                        return Some((ev, false));
                    }
                    let kind = if pre {
                        EventKind::CommandRequested
                    } else {
                        EventKind::CommandCompleted
                    };
                    let mut ev = mk(kind);
                    ev.command = command;
                    Some((ev, pre))
                }
                "apply_patch" | "Edit" | "Write" => {
                    let mut ev = mk(EventKind::FileEdited);
                    ev.content = ti
                        .get("command")
                        .and_then(|c| c.as_str())
                        .map(str::to_string)
                        .or_else(|| {
                            ti.get("patchText")
                                .and_then(|c| c.as_str())
                                .map(str::to_string)
                        })
                        .or_else(|| {
                            ti.get("content")
                                .and_then(|c| c.as_str())
                                .map(str::to_string)
                        });
                    ev.file = ti
                        .get("file_path")
                        .or_else(|| ti.get("filePath"))
                        .or_else(|| ti.get("path"))
                        .and_then(|f| f.as_str())
                        .map(str::to_string);
                    Some((ev, pre))
                }
                "WebFetch" | "webfetch" => {
                    let kind = if pre {
                        EventKind::CommandRequested
                    } else {
                        EventKind::CommandCompleted
                    };
                    let mut ev = mk(kind);
                    ev.command = ti.get("url").and_then(|u| u.as_str()).map(str::to_string);
                    Some((ev, pre))
                }
                other => {
                    if !pre {
                        return None;
                    }
                    let mut ev = mk(EventKind::ToolRequested);
                    ev.command = Some(other.to_string());
                    let input = ti.to_string();
                    let input: String = input.chars().take(500).collect();
                    ev.content = Some(format!("{other} {input}"));
                    Some((ev, true))
                }
            }
        }
        "Stop" => Some((mk(EventKind::CompletionRequested), true)),
        "UserPromptSubmit" => {
            let mut ev = mk(EventKind::PromptSubmitted);
            ev.content = v.get("prompt").and_then(|p| p.as_str()).map(str::to_string);
            Some((ev, false))
        }
        "SessionStart" => Some((mk(EventKind::SessionStarted), false)),
        _ => None,
    }
}

/// Is this Bash command a known test runner? Used to turn a PostToolUse Bash
/// completion into durable `test.completed` evidence (evidence-capture port).
/// Substring match on the lowercased command — no regex dependency, and a false
/// positive only records an extra (harmless) test.completed; a false negative
/// just misses one auto-recording.
fn is_test_runner(command: &str) -> bool {
    let c = command.to_lowercase();
    const RUNNERS: &[&str] = &[
        "flutter test",
        "dart test",
        "cargo test",
        "cargo nextest",
        "pytest",
        "go test",
        "npm test",
        "npm run test",
        "yarn test",
        "pnpm test",
        "jest",
        "vitest",
        "phpunit",
        "rspec",
    ];
    RUNNERS.iter().any(|r| c.contains(r))
}

/// Builds the `content` of a synthesized `test.completed` event from the hook's
/// `tool_response`. The exit code is authoritative (jflow's rule); when it is
/// absent, fall back to failure signatures in the output. The word `FAILED` is
/// present in the content exactly when the run failed, so a companion rule
/// (`builtin:text.contains { value: FAILED }`) classifies it Invalid.
fn test_outcome_content(v: &serde_json::Value) -> String {
    let tr = v.get("tool_response");
    let exit = tr.and_then(|r| {
        r.get("exit_code")
            .or_else(|| r.get("exitCode"))
            .or_else(|| r.get("code"))
    });
    let output = tr
        .and_then(|r| {
            r.get("stdout")
                .or_else(|| r.get("output"))
                .or_else(|| r.get("stderr"))
        })
        .and_then(|o| o.as_str())
        .unwrap_or("");

    let failed = match exit.and_then(serde_json::Value::as_i64) {
        Some(0) => false,
        Some(_) => true,
        // No exit code reported: read the output for a failure signature.
        None => {
            let o = output.to_lowercase();
            o.contains("failed")
                || o.contains("panicked")
                || o.contains("error:")
                || o.contains("test result: fail")
        }
    };

    if failed {
        format!("FAILED\n{output}")
    } else {
        format!("passed\n{output}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_detection_covers_the_common_ecosystems() {
        assert!(is_test_runner("cargo test --workspace"));
        assert!(is_test_runner("TZ=UTC flutter test"));
        assert!(is_test_runner("pytest -q"));
        assert!(is_test_runner("npm test"));
        // Not a test runner → stays a plain command.completed.
        assert!(!is_test_runner("cargo build"));
        assert!(!is_test_runner("git commit -m x"));
        assert!(!is_test_runner("echo hi > f.txt"));
    }

    #[test]
    fn a_failing_test_run_becomes_test_completed_evidence_marked_failed() {
        // PostToolUse Bash of `cargo test` that exited non-zero → a
        // `test.completed` event whose content contains FAILED (so a
        // record-test-result rule classifies it Invalid) and is feedback-only.
        let payload = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "session_id": "s1",
            "tool_name": "Bash",
            "tool_input": { "command": "cargo test --workspace" },
            "tool_response": { "exit_code": 101, "stdout": "test result: FAILED. 1 failed" }
        })
        .to_string();

        let (event, preventable) = parse_claude_code_hook(&payload).expect("parsed");
        assert_eq!(event.kind, EventKind::TestCompleted);
        assert!(
            !preventable,
            "a completed run is post-hoc feedback, never a block"
        );
        assert!(
            event.content.as_deref().unwrap_or("").contains("FAILED"),
            "a non-zero exit must surface as FAILED for the classifying rule"
        );
    }

    #[test]
    fn a_passing_test_run_is_test_completed_without_failed_marker() {
        let payload = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "session_id": "s1",
            "tool_name": "Bash",
            "tool_input": { "command": "cargo test" },
            "tool_response": { "exit_code": 0, "stdout": "test result: ok. 12 passed" }
        })
        .to_string();

        let (event, _) = parse_claude_code_hook(&payload).expect("parsed");
        assert_eq!(event.kind, EventKind::TestCompleted);
        assert!(!event.content.as_deref().unwrap_or("").contains("FAILED"));
    }

    #[test]
    fn a_user_prompt_becomes_a_non_blocking_prompt_submitted_carrying_the_text() {
        let payload = serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "s1",
            "prompt": "explain https://linear.app/acme/issue/ABC-123/x"
        })
        .to_string();

        let (event, preventable) = parse_claude_code_hook(&payload).expect("parsed");
        assert_eq!(event.kind, EventKind::PromptSubmitted);
        assert!(!preventable, "a prompt is enriched, never blocked");
        assert_eq!(
            event.content.as_deref(),
            Some("explain https://linear.app/acme/issue/ABC-123/x")
        );
    }

    #[test]
    fn a_webfetch_becomes_a_preventable_command_requested_carrying_the_url() {
        // A PreToolUse WebFetch surfaces its URL as `command` on a preventable
        // command.requested, so a rule can force a governed tool instead of a
        // direct read.
        let payload = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "session_id": "s1",
            "tool_name": "WebFetch",
            "tool_input": { "url": "https://linear.app/acme/issue/ABC-123/x" }
        })
        .to_string();

        let (event, preventable) = parse_claude_code_hook(&payload).expect("parsed");
        assert_eq!(event.kind, EventKind::CommandRequested);
        assert!(preventable, "a PreToolUse fetch can be prevented");
        assert_eq!(
            event.command.as_deref(),
            Some("https://linear.app/acme/issue/ABC-123/x")
        );
    }

    #[test]
    fn a_non_test_bash_completion_stays_command_completed() {
        let payload = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "session_id": "s1",
            "tool_name": "Bash",
            "tool_input": { "command": "cargo build" },
            "tool_response": { "exit_code": 0 }
        })
        .to_string();

        let (event, _) = parse_claude_code_hook(&payload).expect("parsed");
        assert_eq!(event.kind, EventKind::CommandCompleted);
    }

    #[test]
    fn an_unmapped_pretooluse_tool_becomes_observe_only_tool_requested() {
        // Catch-all matcher (D-016): a native MCP tool the model is about to use
        // is SEEN as `tool.requested` — carried so keel can deliver context at
        // that moment — but it is observe-only (never a hard block by itself).
        let payload = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "session_id": "s1",
            "tool_name": "mcp__linear-server__get_issue",
            "tool_input": { "id": "ABC-123" }
        })
        .to_string();

        let (event, preventable) = parse_claude_code_hook(&payload).expect("parsed");
        assert_eq!(event.kind, EventKind::ToolRequested);
        assert_eq!(
            event.command.as_deref(),
            Some("mcp__linear-server__get_issue")
        );
        assert!(
            event.content.as_deref().unwrap_or("").contains("linear"),
            "the moment text carries the tool + input so routing can match it"
        );
        assert!(
            preventable,
            "seen before the call, so a rule COULD gate it — default is observe"
        );
    }

    #[test]
    fn an_unmapped_posttooluse_tool_is_ignored() {
        // A completed unmapped tool is post-hoc: nothing to deliver or prevent.
        let payload = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "session_id": "s1",
            "tool_name": "mcp__linear-server__get_issue",
            "tool_input": { "id": "ABC-123" }
        })
        .to_string();
        assert!(parse_claude_code_hook(&payload).is_none());
    }

    #[test]
    fn session_start_becomes_a_non_blocking_session_started() {
        let payload = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "s1",
            "source": "startup"
        })
        .to_string();

        let (event, preventable) = parse_claude_code_hook(&payload).expect("parsed");
        assert_eq!(event.kind, EventKind::SessionStarted);
        assert!(!preventable, "session start delivers context, never blocks");
    }

    #[test]
    fn codex_apply_patch_becomes_preventable_file_edited() {
        let payload = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "session_id": "s1",
            "tool_name": "apply_patch",
            "tool_input": { "command": "*** Update File: src/lib.rs\n+fn f() {}\n" }
        })
        .to_string();

        let (event, preventable) = parse_codex_hook(&payload).expect("parsed");
        assert_eq!(event.kind, EventKind::FileEdited);
        assert!(preventable);
        assert!(event.content.unwrap().contains("src/lib.rs"));
    }

    #[test]
    fn codex_stop_becomes_preventable_completion_requested() {
        let payload = serde_json::json!({
            "hook_event_name": "Stop",
            "session_id": "s1",
            "last_assistant_message": "done"
        })
        .to_string();

        let (event, preventable) = parse_codex_hook(&payload).expect("parsed");
        assert_eq!(event.kind, EventKind::CompletionRequested);
        assert!(preventable);
    }
}
