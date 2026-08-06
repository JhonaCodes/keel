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
                .append(&entry(&format!("ev_h{i}"), "r-hot", Verdict::Invalid, "2026-01-02T00:00:00Z"))
                .unwrap();
        }
        for i in 0..10 {
            ledger
                .append(&entry(&format!("ev_d{i}"), "r-dead", Verdict::Valid, "2026-01-03T00:00:00Z"))
                .unwrap();
        }
        for i in 0..4 {
            ledger
                .append(&entry(&format!("ev_f{i}"), "r-fuzzy", Verdict::Unknown, "2026-01-04T00:00:00Z"))
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
                .append(&entry(&format!("ev_{i}"), "r1", Verdict::Invalid, "2026-01-01T00:00:00Z"))
                .unwrap();
        }
        ledger
            .append(&entry("ev_other", "r2", Verdict::Invalid, "2026-01-01T00:00:00Z"))
            .unwrap();

        let osc = ledger.oscillations(3).unwrap();
        assert_eq!(osc.len(), 1);
        assert_eq!(osc[0].rule_id, "r1");
        assert_eq!(osc[0].count, 3);
    }

    #[test]
    fn human_decisions_are_recorded_with_human_class() {
        let ledger = Ledger::open_in_memory().unwrap();
        ledger
            .record_human_decision("r-dead", "prune", "jhonatan", Some("0 fires in P6M"), "hd_1", "2026-01-05T00:00:00Z")
            .unwrap();
        let ds = ledger.human_decisions(Some("r-dead")).unwrap();
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].decision, "prune");
        assert_eq!(ds[0].decided_by, "jhonatan");
    }
