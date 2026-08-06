// SPDX-License-Identifier: Apache-2.0
//! Integration tests for the L3 specialized-agent path (spec section 14) through the
//! public `keel_engine::audit` API — a real executor subprocess, then result
//! validation and ledger recording. Complements the in-crate unit tests that
//! reach `audit`'s private helpers.

use keel_core::{OriginClass, Verdict};
use keel_engine::audit::{AgentSpec, ExecutorSpec, run_audit};
use keel_engine::ledger::Ledger;

fn agent() -> AgentSpec {
    AgentSpec {
        id: "a".into(),
        role: "audit".into(),
        objective: None,
        timeout_ms: 2000,
        max_tokens: None,
    }
}

/// An agent with a declared token ceiling (invariant 13).
fn budgeted_agent(max_tokens: u64) -> AgentSpec {
    AgentSpec {
        max_tokens: Some(max_tokens),
        ..agent()
    }
}

/// section 14.8: a real executor process returning a schema-valid `invalid` verdict is
/// recorded as `semantic` and mapped to `review` — never a block (section 4.7).
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
        None,
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
        None,
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

/// An outputSchema that requires a `findings` array. Feeds the two tests below.
fn schema_requiring_findings() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["verdict", "findings"],
        "properties": {
            "verdict": { "type": "string" },
            "findings": { "type": "array" }
        }
    })
}

/// Invariant 12: a result that parses as a verdict but VIOLATES the declared
/// outputSchema (here: missing the required `findings`) is downgraded to
/// `unknown` with an explicit finding — never trusted.
#[test]
fn result_violating_output_schema_maps_to_unknown() {
    let ledger = Ledger::open_in_memory().unwrap();
    let executor = ExecutorSpec {
        id: "echo".into(),
        // Parses as a verdict, but has no `findings` → violates the schema.
        command: vec![
            "sh".into(),
            "-c".into(),
            r#"cat >/dev/null; echo '{"verdict":"valid"}'"#.into(),
        ],
        model: Some("stub".into()),
        timeout_ms: Some(2000),
    };
    let schema = schema_requiring_findings();
    let out = run_audit(
        &agent(),
        &executor,
        "some diff",
        "sha256:x",
        Some("s1"),
        &ledger,
        "ev_it3".into(),
        "2026-01-01T00:00:00Z".into(),
        Some(&schema),
    );
    assert_eq!(out.verdict, Verdict::Unknown);
    assert!(
        out.findings.iter().any(|f| f.contains("outputSchema")),
        "a schema violation must be reported: {:?}",
        out.findings
    );
}

/// The converse: a result that conforms to the outputSchema keeps its verdict.
#[test]
fn result_matching_output_schema_is_trusted() {
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
    let schema = schema_requiring_findings();
    let out = run_audit(
        &agent(),
        &executor,
        "some diff",
        "sha256:x",
        Some("s1"),
        &ledger,
        "ev_it4".into(),
        "2026-01-01T00:00:00Z".into(),
        Some(&schema),
    );
    assert_eq!(
        out.verdict,
        Verdict::Invalid,
        "a conforming result keeps its verdict"
    );
}

/// Invariant 13: a real executor that reports usage OVER the declared
/// `maxTokens` is not trusted — the verdict is downgraded to `unknown` with an
/// explicit finding, and the ledger records the REAL token count (not 0).
#[test]
fn real_executor_over_budget_maps_to_unknown_and_records_real_tokens() {
    let ledger = Ledger::open_in_memory().unwrap();
    let executor = ExecutorSpec {
        id: "echo".into(),
        // A schema-valid `valid` verdict, but reporting 500 tokens.
        command: vec![
            "sh".into(),
            "-c".into(),
            r#"cat >/dev/null; echo '{"verdict":"valid","findings":[],"tokens":500}'"#.into(),
        ],
        model: Some("stub".into()),
        timeout_ms: Some(2000),
    };
    let out = run_audit(
        &budgeted_agent(100),
        &executor,
        "some diff",
        "sha256:x",
        Some("s1"),
        &ledger,
        "ev_it5".into(),
        "2026-01-01T00:00:00Z".into(),
        None,
    );
    assert_eq!(out.verdict, Verdict::Unknown, "over-budget is not trusted");
    assert!(
        out.findings.iter().any(|f| f.contains("maxTokens")),
        "the budget breach must be reported: {:?}",
        out.findings
    );
    let entry = ledger.get("ev_it5").unwrap().unwrap();
    assert_eq!(entry.tokens, 500, "the real token count is recorded, not 0");
}

/// The converse: usage within budget keeps the verdict and still records the
/// real token count (closing the `tokens: 0` gap for reporting executors).
#[test]
fn real_executor_within_budget_records_real_tokens() {
    let ledger = Ledger::open_in_memory().unwrap();
    let executor = ExecutorSpec {
        id: "echo".into(),
        command: vec![
            "sh".into(),
            "-c".into(),
            r#"cat >/dev/null; echo '{"verdict":"invalid","findings":["leak"],"tokens":40}'"#
                .into(),
        ],
        model: Some("stub".into()),
        timeout_ms: Some(2000),
    };
    let out = run_audit(
        &budgeted_agent(100),
        &executor,
        "some diff",
        "sha256:x",
        Some("s1"),
        &ledger,
        "ev_it6".into(),
        "2026-01-01T00:00:00Z".into(),
        None,
    );
    assert_eq!(
        out.verdict,
        Verdict::Invalid,
        "within budget keeps the verdict"
    );
    let entry = ledger.get("ev_it6").unwrap().unwrap();
    assert_eq!(entry.tokens, 40, "real tokens recorded within budget");
}

/// section 4.6 / section 14.8: a real executor that never returns within its
/// timeout is `unknown` — the engine kills it and records honestly, it does not
/// hang or crash.
#[test]
fn real_executor_timeout_maps_to_unknown() {
    let ledger = Ledger::open_in_memory().unwrap();
    let executor = ExecutorSpec {
        id: "slow".into(),
        command: vec![
            "sh".into(),
            "-c".into(),
            r#"cat >/dev/null; sleep 5; echo '{"verdict":"valid","findings":[]}'"#.into(),
        ],
        model: Some("stub".into()),
        timeout_ms: Some(100),
    };
    let out = run_audit(
        &agent(),
        &executor,
        "some diff",
        "sha256:x",
        Some("s1"),
        &ledger,
        "ev_it7".into(),
        "2026-01-01T00:00:00Z".into(),
        None,
    );
    assert_eq!(
        out.verdict,
        Verdict::Unknown,
        "a timed-out executor is unknown"
    );
    assert!(
        out.findings.iter().any(|f| f.contains("timed out")),
        "the timeout must be reported: {:?}",
        out.findings
    );
}
