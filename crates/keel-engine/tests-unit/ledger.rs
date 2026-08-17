// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `ledger` (relocated out of src; included via #[path] in src/ledger.rs).

use super::*;

fn entry(id: &str, rule: &str, verdict: Verdict, ts: &str) -> LedgerEntry {
    LedgerEntry {
        id: id.into(),
        ts: ts.into(),
        session_id: Some("s1".into()),
        snapshot_hash: "sha256:x".into(),
        rule_id: rule.into(),
        rule_version: Some("1.0.0".into()),
        event_kind: EventKind::FileEdited,
        verdict,
        origin: OriginClass::Deterministic,
        declared_decision: Decision::Block,
        effective_decision: Decision::Review,
        latency_ms: 5,
        tokens: 0,
        file: Some("lib/a.dart".into()),
        line: Some(10),
        detail: None,
    }
}

#[test]
fn append_and_get_roundtrip() {
    let ledger = Ledger::open_in_memory().unwrap();
    let e = entry("ev_1", "r1", Verdict::Invalid, "2026-01-01T00:00:00Z");
    ledger.append(&e).unwrap();
    let back = ledger.get("ev_1").unwrap().unwrap();
    assert_eq!(back.rule_id, "r1");
    assert_eq!(back.verdict, Verdict::Invalid);
    assert_eq!(back.declared_decision, Decision::Block);
    assert_eq!(back.effective_decision, Decision::Review);
    assert_eq!(back.origin, OriginClass::Deterministic);
}

/// The telemetry of section 6.4 is THE PRODUCT — it gets tested as a product.
#[test]
fn rule_stats_answer_the_operational_questions() {
    let ledger = Ledger::open_in_memory().unwrap();
    // r-hot always fires; r-dead never; r-fuzzy comes back unknown.
    for i in 0..5 {
        ledger
            .append(&entry(
                &format!("ev_h{i}"),
                "r-hot",
                Verdict::Invalid,
                "2026-01-02T00:00:00Z",
            ))
            .unwrap();
    }
    for i in 0..10 {
        ledger
            .append(&entry(
                &format!("ev_d{i}"),
                "r-dead",
                Verdict::Valid,
                "2026-01-03T00:00:00Z",
            ))
            .unwrap();
    }
    for i in 0..4 {
        ledger
            .append(&entry(
                &format!("ev_f{i}"),
                "r-fuzzy",
                Verdict::Unknown,
                "2026-01-04T00:00:00Z",
            ))
            .unwrap();
    }

    let stats = ledger.rule_stats().unwrap();
    let by_id = |id: &str| stats.iter().find(|s| s.rule_id == id).unwrap();

    let hot = by_id("r-hot");
    assert_eq!((hot.evaluations, hot.invalid), (5, 5)); // the pattern is wrong, not the devs
    let dead = by_id("r-dead");
    assert_eq!((dead.evaluations, dead.invalid), (10, 0)); // prune candidate
    let fuzzy = by_id("r-fuzzy");
    assert_eq!((fuzzy.evaluations, fuzzy.unknown), (4, 4)); // mis-specified
}

/// section 6.5 oscillation: same rule+location repeated within a session.
#[test]
fn oscillation_detects_repeated_findings_at_same_location() {
    let ledger = Ledger::open_in_memory().unwrap();
    for i in 0..3 {
        ledger
            .append(&entry(
                &format!("ev_{i}"),
                "r1",
                Verdict::Invalid,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();
    }
    ledger
        .append(&entry(
            "ev_other",
            "r2",
            Verdict::Invalid,
            "2026-01-01T00:00:00Z",
        ))
        .unwrap();

    let osc = ledger.oscillations(3).unwrap();
    assert_eq!(osc.len(), 1);
    assert_eq!(osc[0].rule_id, "r1");
    assert_eq!(osc[0].count, 3);
}

/// Read side of the `evidence.recorded` precondition: distinct (event_kind,
/// verdict) pairs for a session, filtered by session, deduplicated even when
/// the same pair fires many times.
#[test]
fn recorded_evidence_is_distinct_and_scoped_to_the_session() {
    let ledger = Ledger::open_in_memory().unwrap();
    // s1: test.completed/invalid fires 3 times — must collapse to one pair.
    for i in 0..3 {
        ledger
            .append(&LedgerEntry {
                event_kind: EventKind::TestCompleted,
                ..entry(
                    &format!("ev_s1_{i}"),
                    "r1",
                    Verdict::Invalid,
                    "2026-01-01T00:00:00Z",
                )
            })
            .unwrap();
    }
    // s1: a second, distinct pair.
    ledger
        .append(&LedgerEntry {
            event_kind: EventKind::FileEdited,
            ..entry("ev_s1_other", "r2", Verdict::Valid, "2026-01-01T00:00:01Z")
        })
        .unwrap();
    // s2: same event_kind/verdict as s1's first pair, but a DIFFERENT session
    // — must not leak into s1's result.
    ledger
        .append(&LedgerEntry {
            event_kind: EventKind::TestCompleted,
            session_id: Some("s2".into()),
            ..entry("ev_s2", "r1", Verdict::Invalid, "2026-01-01T00:00:02Z")
        })
        .unwrap();

    let mut s1 = ledger.recorded_evidence("s1").unwrap();
    s1.sort_by_key(|(k, v, r)| (format!("{k:?}"), format!("{v:?}"), r.clone()));
    assert_eq!(
        s1,
        vec![
            (EventKind::FileEdited, Verdict::Valid, "r2".to_string()),
            (EventKind::TestCompleted, Verdict::Invalid, "r1".to_string()),
        ]
    );

    let s2 = ledger.recorded_evidence("s2").unwrap();
    assert_eq!(
        s2,
        vec![(EventKind::TestCompleted, Verdict::Invalid, "r1".to_string())]
    );

    let unknown = ledger.recorded_evidence("no-such-session").unwrap();
    assert!(unknown.is_empty());
}

/// Two recorder rules classifying the SAME event kind with the same verdict are
/// distinct evidence, not one collapsed row — otherwise a gate demanding one of
/// them would be satisfied by the other.
#[test]
fn recorded_evidence_keeps_recorders_apart() {
    let ledger = Ledger::open_in_memory().unwrap();
    for (id, rule) in [
        ("ev_impl", "global.record-audit"),
        ("ev_test", "global.record-test-audit"),
    ] {
        ledger
            .append(&LedgerEntry {
                event_kind: EventKind::TaskCompleted,
                ..entry(id, rule, Verdict::Invalid, "2026-01-01T00:00:00Z")
            })
            .unwrap();
    }

    let mut got = ledger.recorded_evidence("s1").unwrap();
    got.sort_by_key(|(_, _, r)| r.clone());
    assert_eq!(
        got,
        vec![
            (
                EventKind::TaskCompleted,
                Verdict::Invalid,
                "global.record-audit".to_string()
            ),
            (
                EventKind::TaskCompleted,
                Verdict::Invalid,
                "global.record-test-audit".to_string()
            ),
        ]
    );
}

#[test]
fn recorded_audits_are_scoped_and_ignore_unstructured_go_text() {
    let ledger = Ledger::open_in_memory().unwrap();
    ledger
        .append(&LedgerEntry {
            event_kind: EventKind::TaskCompleted,
            detail: Some("VERDICT: GO | AUDIT_EVIDENCE: {\"verdict\":\"GO\",\"scope\":\"sha256:new\",\"mode\":\"focused\",\"files\":[\"lib/a.dart\"]}".into()),
            ..entry("ev_audit", "record-audit", Verdict::Invalid, "2026-01-01T00:00:00Z")
        })
        .unwrap();
    ledger
        .append(&LedgerEntry {
            event_kind: EventKind::TaskCompleted,
            detail: Some("VERDICT: GO".into()),
            ..entry(
                "ev_old",
                "record-audit",
                Verdict::Invalid,
                "2026-01-01T00:00:01Z",
            )
        })
        .unwrap();

    let audits = ledger.recorded_audits("s1").unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].scope, "sha256:new");
    assert_eq!(audits[0].mode, "focused");
}

#[test]
fn audits_for_scope_find_evidence_recorded_under_any_session() {
    // The scope is the hash of the patch: it identifies the change-set, not the
    // observer. A GO landing under a different session id than the one the gate
    // evaluates used to be swallowed silently — recorded, and still blocking.
    let ledger = Ledger::open_in_memory().unwrap();
    ledger
        .append(&LedgerEntry {
            event_kind: EventKind::TaskCompleted,
            session_id: Some("some-other-session".into()),
            detail: Some("VERDICT: GO | AUDIT_EVIDENCE: {\"verdict\":\"GO\",\"scope\":\"sha256:target\",\"mode\":\"extended\",\"files\":[\"lib/a.dart\"]}".into()),
            ..entry("ev_other", "record-audit", Verdict::Invalid, "2026-01-01T00:00:00Z")
        })
        .unwrap();
    ledger
        .append(&LedgerEntry {
            event_kind: EventKind::TaskCompleted,
            session_id: Some("some-other-session".into()),
            detail: Some("VERDICT: GO | AUDIT_EVIDENCE: {\"verdict\":\"GO\",\"scope\":\"sha256:unrelated\",\"mode\":\"focused\",\"files\":[]}".into()),
            ..entry("ev_unrelated", "record-audit", Verdict::Invalid, "2026-01-01T00:00:01Z")
        })
        .unwrap();

    // Session-keyed lookup misses it — that was the whole failure mode.
    assert!(ledger.recorded_audits("s1").unwrap().is_empty());

    let found = ledger.audits_for_scope("sha256:target").unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].scope, "sha256:target");
    assert_eq!(found[0].mode, "extended");

    // Still keyed on the change-set: another scope is not evidence for this one.
    assert!(ledger.audits_for_scope("sha256:absent").unwrap().is_empty());
}

#[test]
fn human_decisions_are_recorded_with_human_class() {
    let ledger = Ledger::open_in_memory().unwrap();
    ledger
        .record_human_decision(
            "r-dead",
            "prune",
            "jhonatan",
            Some("0 fires in P6M"),
            "hd_1",
            "2026-01-05T00:00:00Z",
        )
        .unwrap();
    let ds = ledger.human_decisions(Some("r-dead")).unwrap();
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0].decision, "prune");
    assert_eq!(ds[0].decided_by, "jhonatan");
}
