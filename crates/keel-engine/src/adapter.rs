// SPDX-License-Identifier: Apache-2.0
//! Adapter capability manifest + preflight (spec section 12.1, invariant 8).
//!
//! A blocking policy is only real if the client can actually prevent the
//! action. This module lets an adapter DECLARE what it can prevent, and lets
//! the compiler REJECT a `block` the client cannot honor — instead of assuming
//! it and shipping a false promise (invariant 8). The check is static: it reads
//! the snapshot, never runs anything.

use crate::snapshot::CompiledRule;
use crate::snapshot::Snapshot;
use keel_core::event::EventKind;
use keel_core::Decision;
use std::collections::BTreeSet;

/// What a client adapter can enforce (section 12.1). Minimal Phase-1 form: the set of
/// event kinds on which the adapter can PREVENT the action before it runs. Any
/// other event may still be delivered as post-hoc feedback, never prevention.
#[derive(Debug, Clone)]
pub struct AdapterManifest {
    pub id: String,
    pub blockable: BTreeSet<EventKind>,
}

impl AdapterManifest {
    /// The Claude Code adapter: `exit 2` prevents the action only on the hooks
    /// that fire BEFORE it — `PreToolUse(Bash)` → `command.requested` and
    /// `Stop` → `completion.requested`. It exposes no hook for transition or
    /// delivery requests, so it cannot prevent those (they map to nothing).
    pub fn claude_code() -> Self {
        AdapterManifest {
            id: "claude-code".into(),
            blockable: [EventKind::CommandRequested, EventKind::CompletionRequested]
                .into_iter()
                .collect(),
        }
    }

    /// Known adapters. Unknown client ids return `None`.
    pub fn for_client(id: &str) -> Option<Self> {
        match id {
            "claude-code" => Some(Self::claude_code()),
            _ => None,
        }
    }
}

/// A policy the target adapter cannot honor (invariant 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightViolation {
    pub rule_id: String,
    pub event_kind: EventKind,
    pub reason: String,
}

/// Preflight: every rule that declares a `block` on an event whose prevention
/// the adapter cannot deliver (an inner-ring or completion request the adapter
/// cannot stop) is a false promise. Outer-ring blocks (e.g. `file.edited`) are
/// fine — the runtime already downgrades those to feedback (section 5.3), so they are
/// not flagged.
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
                        "adapter `{}` cannot prevent `{}` — a `block` here would be a false promise (invariant 8)",
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
