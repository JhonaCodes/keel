// SPDX-License-Identifier: Apache-2.0
//! Tool Runner — deterministic builtins and wrapper for external tools.
//!
//! Contract (spec section 4.4/section 4.6): the rule declares; the tool implements; the
//! tool is CODE. It runs on CPU, not in a model: a deterministic validation
//! costs 0 tokens and fires identically on every run. Its output is honest:
//! `valid | invalid | unknown` — it is never forced to decide.
//!
//! BOUNDARY RULE: this module does not import `keel_dsl` (it operates on the
//! COMPILED types from the snapshot).
//!
//! FAIL-SAFE (section 6.4): a broken tool (timeout, missing binary, unparseable
//! output) produces `unknown`, NEVER an engine crash. That `unknown` feeds
//! the "misspecified rule" telemetry metric — it is data, not an error.

use crate::snapshot::{
    CompiledPrecondition, CompiledToolCall, CompiledToolRef, ExternalToolDef, OutputKind,
};
use keel_core::event::{Event, EventKind};
use keel_core::{OriginClass, Verdict};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Tool execution context. Manifests preserve commands with relative paths
/// (the snapshot contains no machine paths — invariant 9); they are resolved
/// here: the subprocess runs with `cwd = workspace_root`.
#[derive(Debug, Clone, Copy)]
pub struct ExecContext<'a> {
    pub workspace_root: &'a Path,
}

/// Structured output of a tool (three states + cost, spec section 6.4).
#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub verdict: Verdict,
    /// In Phase 0 always `Deterministic`: no llm-evaluator is executed.
    pub origin: OriginClass,
    pub findings: Vec<Finding>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// IDs of builtins supported in Phase 0. This is the list the compiler uses
/// to resolve references (spec section 10.1 "Tool validation").
pub const BUILTIN_DETECTORS: &[&str] = &["text.contains", "text.regex", "command.classify"];
pub const BUILTIN_PRECONDITIONS: &[&str] = &[
    "env.present",
    "flag.present",
    "skill.loaded",
    "evidence.recorded",
];

/// Runs a builtin DETECTOR. Returns only hit/no-hit: the detector never
/// decides (section 4.5) — a match opens the door to `validate`, nothing more.
///
/// If the detector fails (missing parameter, broken regex that slipped past
/// the compiler), it OPENS THE DOOR (`true`): failing open here costs one
/// tool execution; failing closed would cost an unevaluated violation.
pub fn run_detector(call: &CompiledToolCall, event: &Event) -> bool {
    let CompiledToolRef::Builtin(id) = &call.using else {
        // The compiler only admits builtin detectors in Phase 0; if something
        // external got here, open the door and let validate decide.
        return true;
    };
    let with = call.with.as_ref();
    match id.as_str() {
        "text.contains" => {
            let Some(needle) = with.and_then(|w| w.get("value")).and_then(|v| v.as_str()) else {
                return true;
            };
            event.content.as_deref().is_some_and(|c| c.contains(needle))
        }
        "text.regex" => {
            let Some(pattern) = with.and_then(|w| w.get("pattern")).and_then(|v| v.as_str()) else {
                return true;
            };
            let Ok(re) = regex::Regex::new(pattern) else {
                return true; // validated at compile time; fail-open just in case
            };
            event.content.as_deref().is_some_and(|c| re.is_match(c))
        }
        "command.classify" => {
            let Some(families) = with
                .and_then(|w| w.get("families"))
                .and_then(|v| v.as_array())
            else {
                return true;
            };
            let Some(cmd) = event.command.as_deref() else {
                return false; // without a command there is nothing to classify
            };
            families
                .iter()
                .filter_map(|f| f.as_str())
                .any(|family| command_matches_family(cmd, family))
        }
        _ => true,
    }
}

/// A family matches if it is the invoked program (first token) or if, as a
/// glob, it matches the full command (e.g. `*/artisan db:*`, section 11.4).
fn command_matches_family(command: &str, family: &str) -> bool {
    if family.contains('*') {
        return globset::Glob::new(family)
            .ok()
            .map(|g| g.compile_matcher().is_match(command))
            .unwrap_or(false);
    }
    command
        .split_whitespace()
        .next()
        .map(|prog| prog == family || prog.ends_with(&format!("/{family}")))
        .unwrap_or(false)
}

/// Evaluates a PRECONDITION (ADR-022): state of the world at the moment of
/// the request. Returns `true` if the condition holds.
///
/// FAIL-CLOSED: a precondition that cannot be evaluated (external tool in
/// `unknown`) FAILS — ADR-022 declares them fail-closed by default on
/// irreversible rules, and Phase 0 applies that default to all of them (it
/// records, does not block, so the conservative bias is free and honest).
pub fn run_precondition(
    pre: &CompiledPrecondition,
    event: &Event,
    tools: &BTreeMap<String, ExternalToolDef>,
    ctx: ExecContext<'_>,
) -> bool {
    match &pre.using {
        CompiledToolRef::Builtin(id) => {
            let with = pre.with.as_ref();
            match id.as_str() {
                "env.present" => {
                    let Some(name) = with.and_then(|w| w.get("name")).and_then(|v| v.as_str())
                    else {
                        return false;
                    };
                    // State of the world CAPTURED IN THE EVENT (honest replay,
                    // see keel-core::event): a governed capability can probe the
                    // real environment in Phase 1.
                    event.env.contains_key(name)
                }
                "flag.present" => {
                    let Some(flag) = with.and_then(|w| w.get("flag")).and_then(|v| v.as_str())
                    else {
                        return false;
                    };
                    event
                        .command
                        .as_deref()
                        .is_some_and(|c| c.split_whitespace().any(|tok| tok == flag))
                }
                "skill.loaded" => {
                    // Hard cognitive gate: the action is allowed only if the
                    // session has already loaded the named skill through keel
                    // (receipt in the store, surfaced on the event by the
                    // broker). onFail → the rule blocks and the packet tells the
                    // model which skill to load. This is how keel FORCES a skill
                    // for a job instead of merely suggesting it.
                    let Some(id) = with.and_then(|w| w.get("id")).and_then(|v| v.as_str()) else {
                        return false;
                    };
                    event.loaded_skills.iter().any(|s| s == id)
                }
                "evidence.recorded" => {
                    // Generic counterpart of `skill.loaded` for any event
                    // kind (not just skills): the action is allowed only if
                    // this session already has a ledger entry for `event`
                    // (and, if given, matching `verdict`). Same source of
                    // truth as skill.loaded — a field the broker/gate
                    // populated on `Event` BEFORE evaluation, never a live
                    // query from inside the runtime (⇏ dsl/store boundary).
                    let Some(target_kind) = with
                        .and_then(|w| w.get("event"))
                        .and_then(|v| serde_json::from_value::<EventKind>(v.clone()).ok())
                    else {
                        return false; // malformed parameter ≠ evidence: fail-closed
                    };
                    let target_verdict = with
                        .and_then(|w| w.get("verdict"))
                        .and_then(|v| serde_json::from_value::<Verdict>(v.clone()).ok());
                    event
                        .recorded_evidence
                        .iter()
                        .any(|(k, v)| *k == target_kind && target_verdict.is_none_or(|tv| tv == *v))
                }
                _ => false,
            }
        }
        CompiledToolRef::External(id) => match tools.get(id) {
            Some(def) => run_external(def, event, ctx).verdict == Verdict::Valid,
            None => false, // no manifest, no evaluation → fail-closed
        },
    }
}

/// Runs a VALIDATE. The real verdict (spec section 4.6).
///
/// Semantics of builtins in validate position: the pattern describes the
/// VIOLATION — match → `invalid`, no-match → `valid`, absent content →
/// `unknown` (undecidable without the input).
pub fn run_validate(
    call: &CompiledToolCall,
    event: &Event,
    tools: &BTreeMap<String, ExternalToolDef>,
    ctx: ExecContext<'_>,
) -> ToolOutcome {
    let started = Instant::now();
    match &call.using {
        CompiledToolRef::Builtin(id) => {
            let verdict = builtin_validate(id, call.with.as_ref(), event);
            ToolOutcome {
                verdict,
                origin: OriginClass::Deterministic,
                findings: vec![],
                latency_ms: started.elapsed().as_millis() as u64,
            }
        }
        CompiledToolRef::External(id) => match tools.get(id) {
            Some(def) => run_external(def, event, ctx),
            None => ToolOutcome {
                verdict: Verdict::Unknown,
                origin: OriginClass::Deterministic,
                findings: vec![Finding {
                    message: format!("tool `{id}` has no manifest in the snapshot"),
                    file: None,
                    line: None,
                }],
                latency_ms: started.elapsed().as_millis() as u64,
            },
        },
    }
}

fn builtin_validate(id: &str, with: Option<&serde_json::Value>, event: &Event) -> Verdict {
    match id {
        "text.contains" => {
            let Some(needle) = with.and_then(|w| w.get("value")).and_then(|v| v.as_str()) else {
                return Verdict::Unknown;
            };
            match event.content.as_deref() {
                Some(c) if c.contains(needle) => Verdict::Invalid,
                Some(_) => Verdict::Valid,
                None => Verdict::Unknown,
            }
        }
        "text.regex" => {
            let Some(pattern) = with.and_then(|w| w.get("pattern")).and_then(|v| v.as_str()) else {
                return Verdict::Unknown;
            };
            let Ok(re) = regex::Regex::new(pattern) else {
                return Verdict::Unknown;
            };
            match event.content.as_deref() {
                Some(c) if re.is_match(c) => Verdict::Invalid,
                Some(_) => Verdict::Valid,
                None => Verdict::Unknown,
            }
        }
        _ => Verdict::Unknown,
    }
}

/// Runs an external tool: subprocess, event as JSON over stdin, timeout,
/// and FAIL-SAFE mapping of the output to three states.
///
/// | situation                                  | verdict |
/// |--------------------------------------------|-----------|
/// | exit ok + conformant output                | per output |
/// | missing binary                             | `unknown` |
/// | timeout (process killed)                   | `unknown` |
/// | unparseable output                         | `unknown` |
pub fn run_external(def: &ExternalToolDef, event: &Event, ctx: ExecContext<'_>) -> ToolOutcome {
    let started = Instant::now();
    let unknown = |msg: String, started: Instant| ToolOutcome {
        verdict: Verdict::Unknown,
        origin: OriginClass::Deterministic,
        findings: vec![Finding {
            message: msg,
            file: None,
            line: None,
        }],
        latency_ms: started.elapsed().as_millis() as u64,
    };

    let mut cmd = Command::new(&def.command[0]);
    cmd.args(&def.command[1..])
        // Relative paths in the manifest resolve against the workspace: the
        // snapshot stays free of machine paths (invariant 9).
        .current_dir(ctx.workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return unknown(
                format!(
                    "tool `{}` not executable ({e}) — `unknown`, never a crash",
                    def.id
                ),
                started,
            );
        }
    };

    // The event travels over stdin as JSON: the tool receives no arguments
    // carrying content (avoids quoting issues and keeps the manifest static).
    if let Some(stdin) = child.stdin.take() {
        let payload = serde_json::to_vec(event).unwrap_or_default();
        let mut stdin = stdin;
        let _ = stdin.write_all(&payload);
        // drop closes stdin → the tool sees EOF.
    }

    // Wait with timeout by polling (std has no wait_timeout).
    let deadline = Duration::from_millis(def.timeout_ms);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return unknown(
                        format!(
                            "tool `{}` exceeded {}ms — timeout → `unknown`",
                            def.id, def.timeout_ms
                        ),
                        started,
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                return unknown(
                    format!("tool `{}` failed while waiting: {e}", def.id),
                    started,
                );
            }
        }
    };

    let stdout = child
        .stdout
        .take()
        .and_then(|mut s| {
            let mut buf = String::new();
            std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
            Some(buf)
        })
        .unwrap_or_default();

    let latency_ms = started.elapsed().as_millis() as u64;
    let (verdict, findings) = match def.output {
        OutputKind::ExitCode => {
            let v = match status.code() {
                Some(0) => Verdict::Valid,
                Some(1) => Verdict::Invalid,
                _ => Verdict::Unknown,
            };
            (v, vec![])
        }
        OutputKind::VerdictJson => match parse_verdict_json(&stdout) {
            Some(r) => r,
            None => {
                return unknown(
                    format!("tool `{}`: unparseable verdict-json output", def.id),
                    started,
                );
            }
        },
        OutputKind::Sarif => match parse_sarif(&stdout) {
            Some(r) => r,
            None => return unknown(format!("tool `{}`: unparseable SARIF", def.id), started),
        },
    };

    ToolOutcome {
        verdict,
        origin: OriginClass::Deterministic,
        findings,
        latency_ms,
    }
}

/// Minimal three-state contract for user-written scripts:
/// `{"verdict": "valid|invalid|unknown", "findings": [{"message", "file?", "line?"}]}`
fn parse_verdict_json(stdout: &str) -> Option<(Verdict, Vec<Finding>)> {
    #[derive(Deserialize)]
    struct Out {
        verdict: Verdict,
        #[serde(default)]
        findings: Vec<Finding>,
    }
    let out: Out = serde_json::from_str(stdout.trim()).ok()?;
    Some((out.verdict, out.findings))
}

/// SARIF (ADR-016): empty `runs[].results[]` → valid; with results →
/// invalid (results are mapped to findings). Unparseable → None (unknown).
fn parse_sarif(stdout: &str) -> Option<(Verdict, Vec<Finding>)> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    let runs = v.get("runs")?.as_array()?;
    let mut findings = Vec::new();
    for run in runs {
        for result in run
            .get("results")
            .and_then(|r| r.as_array())
            .unwrap_or(&vec![])
        {
            let message = result
                .pointer("/message/text")
                .and_then(|m| m.as_str())
                .unwrap_or("SARIF finding without a message")
                .to_string();
            let file = result
                .pointer("/locations/0/physicalLocation/artifactLocation/uri")
                .and_then(|u| u.as_str())
                .map(str::to_string);
            let line = result
                .pointer("/locations/0/physicalLocation/region/startLine")
                .and_then(|l| l.as_u64())
                .map(|l| l as u32);
            findings.push(Finding {
                message,
                file,
                line,
            });
        }
    }
    let verdict = if findings.is_empty() {
        Verdict::Valid
    } else {
        Verdict::Invalid
    };
    Some((verdict, findings))
}

#[cfg(test)]
#[path = "../tests-unit/tools.rs"]
mod tests;
