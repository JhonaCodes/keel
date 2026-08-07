// SPDX-License-Identifier: Apache-2.0
//! Launch adapter manifest + preflight (spec section 12.1, invariant 8).
//!
//! A blocking policy is only real if the runtime can actually prevent the
//! action. In the parent-runtime model, prevention is NOT a promise made by
//! the client's hooks (the model can edit those) — it is a property of the
//! containment keel itself builds around the child CLI: command interposition
//! (PATH shims → broker) and, when available, the OS sandbox. This module lets
//! a launch adapter DECLARE what that containment can prevent, and lets the
//! preflight FLAG a `block` the containment cannot honor — instead of assuming
//! it and shipping a false promise (invariant 8). The check is static: it
//! reads the snapshot, never runs anything.

use crate::snapshot::CompiledRule;
use crate::snapshot::Snapshot;
use keel_core::Decision;
use keel_core::event::EventKind;
use std::collections::BTreeSet;

/// Commands interposed by default in every session: the destructive/system
/// surface of the inner ring. The workspace can extend this per adapter later
/// (section 12.1); removing entries is not offered — weakening containment is
/// not a per-client choice.
pub const DEFAULT_SHIM_COMMANDS: &[&str] = &["rm", "unlink", "mv", "git", "dd", "shred"];

/// How a governed child CLI is launched and what keel's containment around it
/// can prevent (section 12.1). Minimal parent-runtime form: the base command
/// and the set of event kinds for which interposition delivers TRUE
/// pre-action prevention. Any other event may still be delivered as post-hoc
/// feedback, never prevention.
#[derive(Debug, Clone)]
pub struct AdapterManifest {
    pub id: String,
    /// Base argv of the client CLI (empty for `generic`: the operator passes
    /// the full command after `--`).
    pub command: Vec<String>,
    /// Commands interposed via the session shim dir.
    pub shim_commands: Vec<String>,
    pub blockable: BTreeSet<EventKind>,
    /// How keel injects its MCP endpoint into this client at launch, so the
    /// model discovers its governed skills/agents THROUGH keel (section 12).
    /// `None` when keel has no supported wiring for the client (generic): the
    /// operator can still wire it manually.
    pub mcp: Option<McpInjection>,
}

/// How to wire keel's `keel mcp` server into a specific client CLI, and how to
/// tell the model it is governed. Data, not code — a new client is a new entry,
/// not new logic.
#[derive(Debug, Clone)]
pub struct McpInjection {
    /// How the MCP server config reaches the client.
    pub method: McpMethod,
    /// How the "you are governed by keel; consult keel.skills.list" notice
    /// reaches the model.
    pub announce: Announce,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpMethod {
    /// A flag pointing at a written JSON config file, e.g. Claude Code's
    /// `--mcp-config <file>`.
    ConfigFileFlag { flag: String },
    /// An inline `-c key=value` config override, e.g. Codex's
    /// `-c mcp_servers.keel.command=...`.
    ConfigOverrideFlag { flag: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Announce {
    /// A flag carrying extra system-prompt text, e.g.
    /// `--append-system-prompt`.
    SystemPromptFlag { flag: String },
    /// No client flag: keel writes a one-line notice to the PTY at start.
    PtyLine,
}

impl AdapterManifest {
    /// Containment-based prevention, identical for every client: the shim
    /// layer interposes governed commands, so `command.requested` is truly
    /// pre-action — a blocked command never exists as a process. Transition,
    /// delivery and completion requests have no interposition point in a
    /// foreign CLI's lifecycle (v1), so they are NOT blockable: the runtime
    /// degrades them to feedback (section 5.3).
    fn containment(id: &str, command: Vec<String>, mcp: Option<McpInjection>) -> Self {
        AdapterManifest {
            id: id.into(),
            command,
            shim_commands: DEFAULT_SHIM_COMMANDS
                .iter()
                .map(ToString::to_string)
                .collect(),
            blockable: [EventKind::CommandRequested].into_iter().collect(),
            mcp,
        }
    }

    /// Known launch adapters. Unknown client ids return `None` (use
    /// `generic` + an explicit command instead).
    pub fn for_client(id: &str) -> Option<Self> {
        match id {
            "claude" => Some(Self::containment(
                "claude",
                vec!["claude".into()],
                Some(McpInjection {
                    method: McpMethod::ConfigFileFlag {
                        flag: "--mcp-config".into(),
                    },
                    announce: Announce::SystemPromptFlag {
                        flag: "--append-system-prompt".into(),
                    },
                }),
            )),
            "codex" => Some(Self::containment(
                "codex",
                vec!["codex".into()],
                Some(McpInjection {
                    method: McpMethod::ConfigOverrideFlag { flag: "-c".into() },
                    announce: Announce::PtyLine,
                }),
            )),
            // generic: no assumptions about the CLI's flags — convergence is
            // opt-in (the operator wires MCP), the hard rings still apply.
            "generic" => Some(Self::containment("generic", vec![], None)),
            _ => None,
        }
    }
}

/// A policy the active containment cannot honor (invariant 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightViolation {
    pub rule_id: String,
    pub event_kind: EventKind,
    pub reason: String,
}

/// Preflight: every rule that declares a `block` on an event whose prevention
/// the containment cannot deliver (an inner-ring or completion request with no
/// interposition point) is a false promise. Outer-ring blocks (e.g.
/// `file.edited`) are fine — the runtime already downgrades those to feedback
/// (section 5.3), so they are not flagged.
pub fn preflight(snapshot: &Snapshot, manifest: &AdapterManifest) -> Vec<PreflightViolation> {
    let mut out = Vec::new();
    for rule in &snapshot.rules {
        if !rule_declares_block(rule) {
            continue;
        }
        for &kind in &rule.on {
            let prevention_expected =
                kind.is_inner_ring() || kind == EventKind::CompletionRequested;
            if prevention_expected && !manifest.blockable.contains(&kind) {
                out.push(PreflightViolation {
                    rule_id: rule.id.clone(),
                    event_kind: kind,
                    reason: format!(
                        "containment for `{}` cannot prevent `{}` — a `block` here would be a false promise (invariant 8); it degrades to feedback",
                        manifest.id,
                        event_kind_name(kind)
                    ),
                });
            }
        }
    }
    out
}

fn rule_declares_block(rule: &CompiledRule) -> bool {
    let e = &rule.enforcement;
    [&e.invalid, &e.unknown, &e.valid, &e.always]
        .into_iter()
        .flatten()
        .any(|b| b.decision >= Decision::Block)
}

fn event_kind_name(kind: EventKind) -> String {
    serde_json::to_string(&kind)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

#[cfg(test)]
#[path = "../tests-unit/adapter.rs"]
mod tests;
