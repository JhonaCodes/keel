// SPDX-License-Identifier: Apache-2.0
//! Unit tests for the broker (relocated out of src; included via #[path]).
//!
//! Hermetic: hand-built snapshot, builtin detector only, in-memory decisions.
//! The full external-tool path (`tool:` validate) is covered by the
//! integration test in `test/tests/`.

use super::*;
use keel_engine::snapshot::{
    CompiledBranch, CompiledEnforcement, CompiledRule, CompiledToolCall, CompiledToolRef,
};
use std::collections::BTreeMap;

/// A rule that fires on `rm ...` (builtin `command.classify`) and, having no
/// validate, lands on `unknown` → declared `block` (fail-closed shape).
fn rm_block_rule() -> CompiledRule {
    CompiledRule {
        id: "test.no-rm".into(),
        version: None,
        author: "t".into(),
        adr_ref: "adr:ADR-1".into(),
        review_after: "P6M".into(),
        reversibility: None,
        scope: None,
        on: vec![EventKind::CommandRequested],
        when: None,
        detect: Some(CompiledToolCall {
            using: CompiledToolRef::Builtin("command.classify".into()),
            with: Some(serde_json::json!({ "families": ["rm"] })),
        }),
        preconditions: vec![],
        validate: None,
        enforcement: CompiledEnforcement {
            invalid: None,
            unknown: Some(CompiledBranch {
                decision: Decision::Block,
                load_skills: vec![],
                load_capabilities: vec![],
                report_message: Some("rm is governed here".into()),
                invoke_agent: None,
            }),
            valid: None,
            always: None,
        },
        constraints: None,
        origin_layer: None,
        locked_at: None,
    }
}

fn broker_with(rules: Vec<CompiledRule>) -> (Broker, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let snapshot = Snapshot::build(
        rules,
        BTreeMap::new(),
        BTreeMap::new(),
        "2026-01-01T00:00:00Z".into(),
    )
    .unwrap();
    let ledger = Ledger::open(&dir.path().join("ledger.sqlite")).unwrap();
    let broker = Broker::new(
        snapshot,
        ledger,
        dir.path().to_path_buf(),
        "session-test".into(),
    );
    (broker, dir)
}

fn req(argv: &[&str]) -> ShimRequest {
    ShimRequest {
        name: argv[0].to_string(),
        argv: argv.iter().map(ToString::to_string).collect(),
        cwd: None,
    }
}

#[test]
fn governed_command_is_blocked_with_a_packet_and_evidence() {
    let (broker, dir) = broker_with(vec![rm_block_rule()]);
    let resp = broker.decide(&req(&["rm", "notes.md"])).unwrap();
    assert!(!resp.allow, "rm must be blocked by the unknown→block rule");
    let packet = resp.packet.expect("a block always carries a packet");
    assert!(packet.contains("BLOCKED (test.no-rm)"), "packet: {packet}");
    assert!(
        packet.contains("Evidence: ev_"),
        "packet cites evidence: {packet}"
    );

    // The decision left evidence (the broker is the single ledger writer).
    let ledger = Ledger::open(&dir.path().join("ledger.sqlite")).unwrap();
    assert_eq!(ledger.count().unwrap(), 1);
}

#[test]
fn ungoverned_command_is_allowed_silently() {
    let (broker, _dir) = broker_with(vec![rm_block_rule()]);
    let resp = broker.decide(&req(&["ls", "-la"])).unwrap();
    assert!(resp.allow, "the detector must not fire on ls");
    assert!(resp.packet.is_none(), "an allow is silent");
}

#[test]
fn socket_roundtrip_blocks_over_the_wire() {
    let (broker, dir) = broker_with(vec![rm_block_rule()]);
    let socket = dir.path().join("broker.sock");
    let (join, handle) = spawn(broker, &socket).unwrap();

    let mut stream = UnixStream::connect(&socket).unwrap();
    let payload = serde_json::to_string(&req(&["rm", "x.md"])).unwrap();
    stream.write_all(payload.as_bytes()).unwrap();
    stream.write_all(b"\n").unwrap();

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let resp: ShimResponse = serde_json::from_str(&line).unwrap();
    assert!(!resp.allow);
    assert!(resp.packet.unwrap().contains("BLOCKED"));

    handle.stop();
    join.join().unwrap();
}
