// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `sarif` (relocated out of src; included via #[path] in src/sarif.rs).

use super::*;
use keel_core::event::EventKind;
use keel_core::{Decision, OriginClass, Verdict};

#[test]
fn sarif_carries_keel_properties() {
    let entry = LedgerEntry {
        id: "ev_1".into(),
        ts: "2026-01-01T00:00:00Z".into(),
        session_id: None,
        snapshot_hash: "sha256:abc".into(),
        rule_id: "r1".into(),
        rule_version: None,
        event_kind: EventKind::FileEdited,
        verdict: Verdict::Invalid,
        origin: OriginClass::Deterministic,
        declared_decision: Decision::Block,
        effective_decision: Decision::Review,
        latency_ms: 3,
        tokens: 0,
        file: Some("lib/a.dart".into()),
        line: Some(7),
        detail: Some("direct access to notifier".into()),
    };
    let sarif = to_sarif(&[&entry]);
    let result = &sarif["runs"][0]["results"][0];
    assert_eq!(result["ruleId"], "r1");
    assert_eq!(result["level"], "error");
    assert_eq!(result["properties"]["keel"]["origin"], "deterministic");
    assert_eq!(result["properties"]["keel"]["declaredDecision"], "block");
    assert_eq!(result["properties"]["keel"]["effectiveDecision"], "review");
    assert_eq!(
        result["locations"][0]["physicalLocation"]["region"]["startLine"],
        7
    );
}
