// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `runtime` (relocated out of src; included via #[path] in src/runtime.rs).

    use super::*;
    use crate::snapshot::{
        CompiledEnforcement, CompiledScope, CompiledToolCall, CompiledToolRef, Snapshot,
    };
    use keel_core::event::EventKind;
    use std::collections::BTreeMap;

    fn blocking_rule() -> CompiledRule {
        CompiledRule {
            id: "no-todo".into(),
            version: Some("1.0.0".into()),
            author: "t".into(),
            adr_ref: "adr:ADR-1".into(),
            review_after: "P6M".into(),
            reversibility: Some(keel_core::Reversibility::Reversible),
            scope: Some(CompiledScope {
                languages: vec![],
                include: vec!["lib/**".into()],
                exclude: vec![],
            }),
            on: vec![EventKind::FileEdited],
            when: None,
            detect: None,
            preconditions: vec![],
            validate: Some(CompiledToolCall {
                using: CompiledToolRef::Builtin("text.contains".into()),
                with: Some(serde_json::json!({"value": "TODO"})),
            }),
            enforcement: CompiledEnforcement {
                invalid: Some(CompiledBranch {
                    decision: Decision::Block,
                    load_skills: vec![],
                    load_capabilities: vec![],
                    report_message: None,
                    invoke_agent: None,
                }),
                unknown: None,
                valid: Some(CompiledBranch {
                    decision: Decision::Allow,
                    load_skills: vec![],
                    load_capabilities: vec![],
                    report_message: None,
                    invoke_agent: None,
                }),
                always: None,
            },
            constraints: None,
        }
    }

    fn snap(rules: Vec<CompiledRule>) -> Snapshot {
        Snapshot::build(rules, BTreeMap::new(), BTreeMap::new(), "2026-01-01T00:00:00Z".into())
            .unwrap()
    }

    fn edited(file: &str, content: &str) -> Event {
        Event {
            kind: EventKind::FileEdited,
            session_id: None,
            file: Some(file.into()),
            language: None,
            content: Some(content.into()),
            line: None,
            command: None,
            env: Default::default(),
            files: vec![],
        }
    }

    /// THE L1 test (§5.3 inner ring): in Enforce mode the declared decision
    /// applies as-is — block means BLOCK. This is the keel holding.
    #[test]
    fn enforce_applies_declared_decision() {
        let s = snap(vec![blocking_rule()]);
        let evs = evaluate_event(&s, &edited("lib/a.dart", "x // TODO"), Path::new("."), Mode::Enforce);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].declared_decision, Decision::Block);
        assert_eq!(evs[0].effective_decision, Decision::Block); // no cap
    }

    /// THE Phase 0b test: the declared decision is preserved (block), the
    /// effective one is forced to review. Nothing blocks; everything stays
    /// measurable.
    #[test]
    fn passive_mode_preserves_declared_and_forces_effective_to_review() {
        let s = snap(vec![blocking_rule()]);
        let evs = evaluate_event(&s, &edited("lib/a.dart", "x // TODO"), Path::new("."), Mode::Passive);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].verdict, Verdict::Invalid);
        assert_eq!(evs[0].declared_decision, Decision::Block);
        assert_eq!(evs[0].effective_decision, Decision::Review);
    }

    #[test]
    fn allow_stays_allow_in_passive_mode() {
        let s = snap(vec![blocking_rule()]);
        let evs = evaluate_event(&s, &edited("lib/a.dart", "clean"), Path::new("."), Mode::Passive);
        assert_eq!(evs[0].verdict, Verdict::Valid);
        assert_eq!(evs[0].effective_decision, Decision::Allow);
    }

    #[test]
    fn out_of_scope_rule_does_not_fire() {
        let s = snap(vec![blocking_rule()]);
        let evs = evaluate_event(&s, &edited("src/a.dart", "x // TODO"), Path::new("."), Mode::Passive);
        assert!(evs.is_empty());
    }

    /// A failed precondition = certainty of violation, with the detail of
    /// WHICH one.
    #[test]
    fn failed_precondition_yields_invalid_with_detail() {
        let mut rule = blocking_rule();
        rule.scope = None;
        rule.on = vec![EventKind::CommandRequested];
        rule.validate = None;
        rule.preconditions = vec![crate::snapshot::CompiledPrecondition {
            using: CompiledToolRef::Builtin("env.present".into()),
            with: Some(serde_json::json!({"name": "NUI_PROD_WRITE"})),
            on_fail_declared: Decision::Block,
        }];
        let s = snap(vec![rule]);

        let mut ev = edited("x", "");
        ev.kind = EventKind::CommandRequested;
        ev.file = None;
        ev.content = None;
        ev.command = Some("psql -c 'DELETE FROM t'".into());

        let evs = evaluate_event(&s, &ev, Path::new("."), Mode::Passive);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].verdict, Verdict::Invalid);
        assert_eq!(evs[0].declared_decision, Decision::Block);
        assert_eq!(evs[0].effective_decision, Decision::Review); // passive
        assert!(evs[0].detail.as_deref().unwrap().contains("env.present"));
    }

    /// The unknown branch's invoke is RECORDED, never executed.
    #[test]
    fn unknown_branch_invoke_is_recorded_not_executed() {
        let mut rule = blocking_rule();
        rule.validate = Some(CompiledToolCall {
            using: CompiledToolRef::External("missing.tool".into()),
            with: None,
        });
        rule.enforcement.unknown = Some(CompiledBranch {
            decision: Decision::Review,
            load_skills: vec![],
            load_capabilities: vec![],
            report_message: None,
            invoke_agent: Some("agent:auditor".into()),
        });
        let s = snap(vec![rule]); // no manifest for missing.tool → unknown

        let evs = evaluate_event(&s, &edited("lib/a.dart", "x // TODO"), Path::new("."), Mode::Passive);
        assert_eq!(evs[0].verdict, Verdict::Unknown);
        let detail = evs[0].detail.as_deref().unwrap();
        assert!(detail.contains("NOT executed"), "invoke must be recorded as not executed: {detail}");
        assert_eq!(evs[0].tokens, 0, "no tokens in Phase 0");
    }

    /// A declared `load.capabilities` is a forward-declared field: not yet
    /// activated by the runtime, but surfaced in the evidence so it is never a
    /// silent no-op (like `invoke`, it is recorded honestly).
    #[test]
    fn declared_capabilities_are_surfaced_in_detail() {
        let branch = CompiledBranch {
            decision: Decision::Review,
            load_skills: vec![],
            load_capabilities: vec!["reactive-state.inspect-consumers".into()],
            report_message: None,
            invoke_agent: None,
        };
        let detail = branch_detail(&branch, Verdict::Unknown).unwrap();
        assert!(
            detail.contains("capabilities declared") && detail.contains("inspect-consumers"),
            "declared capabilities must appear in the evidence detail: {detail}"
        );
    }
