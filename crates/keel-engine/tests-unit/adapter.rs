// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `adapter` (relocated out of src; included via #[path] in src/adapter.rs).

use super::*;
use crate::snapshot::{CompiledBranch, CompiledEnforcement, CompiledRule};
use std::collections::BTreeMap;

fn block_rule(id: &str, on: Vec<EventKind>) -> CompiledRule {
    CompiledRule {
        id: id.into(),
        version: None,
        author: "t".into(),
        adr_ref: "adr:ADR-1".into(),
        review_after: "P6M".into(),
        reversibility: None,
        scope: None,
        on,
        when: None,
        detect: None,
        preconditions: vec![],
        validate: None,
        enforcement: CompiledEnforcement {
            invalid: Some(CompiledBranch {
                decision: Decision::Block,
                load_skills: vec![],
                load_capabilities: vec![],
                report_message: None,
                invoke_agent: None,
            }),
            unknown: None,
            valid: None,
            always: None,
        },
        constraints: None,
        origin_layer: None,
        locked_at: None,
    }
}

fn snap(rules: Vec<CompiledRule>) -> Snapshot {
    Snapshot::build(
        rules,
        BTreeMap::new(),
        BTreeMap::new(),
        "2026-01-01T00:00:00Z".into(),
    )
    .unwrap()
}

#[test]
fn claude_code_allows_block_on_command_request() {
    let s = snap(vec![block_rule("r", vec![EventKind::CommandRequested])]);
    let v = preflight(&s, &AdapterManifest::claude_code());
    assert!(v.is_empty(), "command.requested block is honorable: {v:?}");
}

#[test]
fn claude_code_rejects_block_on_transition_request() {
    let s = snap(vec![block_rule("r", vec![EventKind::TransitionRequested])]);
    let v = preflight(&s, &AdapterManifest::claude_code());
    assert_eq!(v.len(), 1, "transition.requested block is a false promise");
    assert_eq!(v[0].event_kind, EventKind::TransitionRequested);
}

#[test]
fn outer_ring_block_is_not_flagged() {
    let s = snap(vec![block_rule("r", vec![EventKind::FileEdited])]);
    let v = preflight(&s, &AdapterManifest::claude_code());
    assert!(
        v.is_empty(),
        "file.edited block downgrades to feedback: {v:?}"
    );
}

#[test]
fn completion_block_is_honorable() {
    let s = snap(vec![block_rule("r", vec![EventKind::CompletionRequested])]);
    assert!(preflight(&s, &AdapterManifest::claude_code()).is_empty());
}
