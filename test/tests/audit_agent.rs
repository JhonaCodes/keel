// SPDX-License-Identifier: Apache-2.0
//! Integration tests for the L3 specialized-agent path (spec §14) through the
//! public `keel_engine::audit` API — a real executor subprocess, then result
//! validation and ledger recording. Complements the in-crate unit tests that
//! reach `audit`'s private helpers.

use keel_core::{OriginClass, Verdict};
use keel_engine::audit::{run_audit, AgentSpec, ExecutorSpec};
use keel_engine::ledger::Ledger;

fn agent() -> AgentSpec {
    AgentSpec {
        id: "a".into(),
        role: "audit".into(),
        objective: None,
        timeout_ms: 2000,
    }
}

/// §14.8: a real executor process returning a schema-valid `invalid` verdict is
/// recorded as `semantic` and mapped to `review` — never a block (§4.7).
#[test]
fn real_executor_invalid_is_semantic_review() {
    let ledger = Ledger::open_in_memory().unwrap();
    let executor = ExecutorSpec {
        id: "echo".into(),
        command: vec![
            "sh".into(),
            "-c".into(),
            r#"cat >/dev/null; echo '{"verdict":"invalid","findings":["leak"]}'"#.into(),
        ],
        model: Some("stub".into()),
        timeout_ms: Some(2000),
    };
    let out = run_audit(
        &agent(),
        &executor,
        "some diff",
        "sha256:x",
        Some("s1"),
        &ledger,
        "ev_it1".into(),
        "2026-01-01T00:00:00Z".into(),
    );
    assert_eq!(out.verdict, Verdict::Invalid);
    let entry = ledger.get("ev_it1").unwrap().unwrap();
    assert_eq!(entry.origin, OriginClass::Semantic);
}

/// Invariant 12: a real executor whose output does NOT validate degrades to
/// `unknown` with an explicit finding — it is never silently accepted.
#[test]
fn real_executor_unvalidated_output_maps_to_unknown() {
    let ledger = Ledger::open_in_memory().unwrap();
    let executor = ExecutorSpec {
        id: "garbage".into(),
        command: vec![
            "sh".into(),
            "-c".into(),
            "cat >/dev/null; echo 'looks fine to me'".into(),
        ],
        model: Some("stub".into()),
        timeout_ms: Some(2000),
    };
    let out = run_audit(
        &agent(),
        &executor,
        "some diff",
        "sha256:x",
        Some("s1"),
        &ledger,
        "ev_it2".into(),
        "2026-01-01T00:00:00Z".into(),
    );
    assert_eq!(out.verdict, Verdict::Unknown);
    assert!(
        out.findings.iter().any(|f| f.contains("did not validate")),
        "an unvalidated agent result must be reported, not swallowed: {:?}",
        out.findings
    );
    let entry = ledger.get("ev_it2").unwrap().unwrap();
    assert_eq!(entry.origin, OriginClass::Semantic);
}
