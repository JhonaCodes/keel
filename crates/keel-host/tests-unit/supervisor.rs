// SPDX-License-Identifier: Apache-2.0
//! Unit tests for the supervisor's oscillation detection (relocated; #[path]).

use super::*;
use keel_core::event::EventKind;
use keel_core::{Decision, OriginClass, Verdict};
use keel_engine::ledger::{Ledger, LedgerEntry};

fn invalid_entry(id: &str, session: &str, rule: &str, file: &str) -> LedgerEntry {
    LedgerEntry {
        id: id.into(),
        ts: "2026-08-07T00:00:00Z".into(),
        session_id: Some(session.into()),
        snapshot_hash: "sha256:x".into(),
        rule_id: rule.into(),
        rule_version: None,
        event_kind: EventKind::CommandRequested,
        verdict: Verdict::Invalid,
        origin: OriginClass::Deterministic,
        declared_decision: Decision::Block,
        effective_decision: Decision::Block,
        latency_ms: 1,
        tokens: 0,
        file: Some(file.into()),
        line: Some(10),
        detail: None,
    }
}

#[test]
fn surfaces_an_oscillation_once_then_stays_quiet() {
    let ledger = Ledger::open_in_memory().unwrap();
    // Three identical invalid findings = an oscillation (threshold 3).
    for i in 0..3 {
        ledger
            .append(&invalid_entry(
                &format!("ev_{i}"),
                "session-a",
                "no-rm",
                "notes.md",
            ))
            .unwrap();
    }

    let mut seen = std::collections::BTreeSet::new();
    let first = new_suggestions(&ledger, "session-a", &mut seen);
    assert_eq!(first.len(), 1, "the oscillation is surfaced: {first:?}");
    assert!(first[0].contains("no-rm"));
    assert!(first[0].contains("notes.md:10"));

    // A second poll with no new oscillation stays silent (no nagging).
    let second = new_suggestions(&ledger, "session-a", &mut seen);
    assert!(second.is_empty(), "already surfaced: {second:?}");
}

#[test]
fn ignores_other_sessions_and_sub_threshold_findings() {
    let ledger = Ledger::open_in_memory().unwrap();
    // Only two findings for session-b: below the oscillation threshold.
    for i in 0..2 {
        ledger
            .append(&invalid_entry(
                &format!("ev_b{i}"),
                "session-b",
                "no-rm",
                "x.md",
            ))
            .unwrap();
    }
    // A full oscillation, but for a DIFFERENT session.
    for i in 0..3 {
        ledger
            .append(&invalid_entry(
                &format!("ev_c{i}"),
                "session-c",
                "no-rm",
                "y.md",
            ))
            .unwrap();
    }

    let mut seen = std::collections::BTreeSet::new();
    assert!(
        new_suggestions(&ledger, "session-b", &mut seen).is_empty(),
        "two findings do not oscillate"
    );
}
