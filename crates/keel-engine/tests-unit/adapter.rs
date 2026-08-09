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
fn containment_allows_block_on_command_request() {
    let s = snap(vec![block_rule("r", vec![EventKind::CommandRequested])]);
    let v = preflight(&s, &AdapterManifest::for_client("claude").unwrap());
    assert!(v.is_empty(), "command.requested block is honorable: {v:?}");
}

#[test]
fn containment_flags_block_on_transition_request() {
    let s = snap(vec![block_rule("r", vec![EventKind::TransitionRequested])]);
    let v = preflight(&s, &AdapterManifest::for_client("claude").unwrap());
    assert_eq!(v.len(), 1, "transition.requested block is a false promise");
    assert_eq!(v[0].event_kind, EventKind::TransitionRequested);
}

#[test]
fn outer_ring_block_is_not_flagged() {
    let s = snap(vec![block_rule("r", vec![EventKind::FileEdited])]);
    let v = preflight(&s, &AdapterManifest::for_client("generic").unwrap());
    assert!(
        v.is_empty(),
        "file.edited block downgrades to feedback: {v:?}"
    );
}

/// v1 parent runtime has no interposition point in a foreign CLI's "done"
/// transition — a completion block must be flagged, not silently promised.
#[test]
fn completion_block_is_flagged_in_v1() {
    let s = snap(vec![block_rule("r", vec![EventKind::CompletionRequested])]);
    let v = preflight(&s, &AdapterManifest::for_client("codex").unwrap());
    assert_eq!(v.len(), 1, "completion.requested is not blockable in v1");
}

#[test]
fn unknown_client_has_no_manifest_and_generic_has_no_base_command() {
    assert!(AdapterManifest::for_client("unknown-cli").is_none());
    let generic = AdapterManifest::for_client("generic").unwrap();
    assert!(generic.command.is_empty());
    assert!(generic.shim_commands.iter().any(|c| c == "rm"));
    // generic makes no assumptions about the client's flags: convergence is
    // opt-in there (no MCP wiring), while the hard rings still apply.
    assert!(generic.mcp.is_none());
}

#[test]
fn opencode_is_a_known_governed_launch_client_without_false_mcp_claims() {
    let opencode = AdapterManifest::for_client("opencode").unwrap();
    assert_eq!(opencode.command, vec!["opencode"]);
    assert!(opencode.shim_commands.iter().any(|c| c == "git"));
    assert!(opencode.mcp.is_none());
    assert!(opencode.hook.is_none());
}

#[test]
fn known_clients_declare_how_to_inject_the_keel_mcp_endpoint() {
    // Convergence wiring is DATA per client, not code.
    assert!(AdapterManifest::for_client("claude").unwrap().mcp.is_some());
    assert!(AdapterManifest::for_client("codex").unwrap().mcp.is_some());
    assert!(
        AdapterManifest::for_client("opencode")
            .unwrap()
            .mcp
            .is_none()
    );
}
