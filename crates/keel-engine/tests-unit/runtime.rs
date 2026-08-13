// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `runtime` (relocated out of src; included via #[path] in src/runtime.rs).

use super::*;
use crate::snapshot::{
    CompiledEnforcement, CompiledPrecondition, CompiledScope, CompiledToolCall, CompiledToolRef,
    Snapshot,
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
        loaded_skills: vec![],
        recorded_evidence: vec![],
    }
}

/// THE L1 test (section 5.3 inner ring): in Enforce mode the declared decision
/// applies as-is — block means BLOCK. This is the keel holding.
#[test]
fn enforce_applies_declared_decision() {
    let s = snap(vec![blocking_rule()]);
    let evs = evaluate_event(
        &s,
        &edited("lib/a.dart", "x // TODO"),
        Path::new("."),
        Mode::Enforce,
    );
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
    let evs = evaluate_event(
        &s,
        &edited("lib/a.dart", "x // TODO"),
        Path::new("."),
        Mode::Passive,
    );
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].verdict, Verdict::Invalid);
    assert_eq!(evs[0].declared_decision, Decision::Block);
    assert_eq!(evs[0].effective_decision, Decision::Review);
}

#[test]
fn allow_stays_allow_in_passive_mode() {
    let s = snap(vec![blocking_rule()]);
    let evs = evaluate_event(
        &s,
        &edited("lib/a.dart", "clean"),
        Path::new("."),
        Mode::Passive,
    );
    assert_eq!(evs[0].verdict, Verdict::Valid);
    assert_eq!(evs[0].effective_decision, Decision::Allow);
}

#[test]
fn out_of_scope_rule_does_not_fire() {
    let s = snap(vec![blocking_rule()]);
    let evs = evaluate_event(
        &s,
        &edited("src/a.dart", "x // TODO"),
        Path::new("."),
        Mode::Passive,
    );
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
        with: Some(serde_json::json!({"name": "PROD_WRITE_ENABLED"})),
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

    let evs = evaluate_event(
        &s,
        &edited("lib/a.dart", "x // TODO"),
        Path::new("."),
        Mode::Passive,
    );
    assert_eq!(evs[0].verdict, Verdict::Unknown);
    let detail = evs[0].detail.as_deref().unwrap();
    assert!(
        detail.contains("NOT executed"),
        "invoke must be recorded as not executed: {detail}"
    );
    assert_eq!(evs[0].tokens, 0, "no tokens in Phase 0");
}

/// Agents are NOT skills (orthogonal paths). On the SAME branch, a `load.skills`
/// ref travels as L2 context to deliver (`eval.load_skills`, pure text, no
/// process), while an `invoke.agent` is only RECORDED as not-executed — the L3
/// executor is spawned solely by the explicit `keel audit` path, never during
/// gate/observe. Neither spawns anything here.
#[test]
fn skills_and_agents_are_orthogonal_paths() {
    let mut rule = blocking_rule();
    rule.validate = Some(CompiledToolCall {
        using: CompiledToolRef::External("missing.tool".into()),
        with: None,
    });
    rule.enforcement.unknown = Some(CompiledBranch {
        decision: Decision::Review,
        load_skills: vec!["skill:access-patterns#compact".into()],
        load_capabilities: vec![],
        report_message: None,
        invoke_agent: Some("agent:auditor".into()),
    });
    let s = snap(vec![rule]);
    let evs = evaluate_event(
        &s,
        &edited("lib/a.dart", "x // TODO"),
        Path::new("."),
        Mode::Passive,
    );
    // L2: the skill ref is carried as context to deliver — no process.
    assert_eq!(
        evs[0].load_skills,
        vec!["skill:access-patterns#compact".to_string()]
    );
    // L3: the agent invoke is only recorded as NOT executed during evaluation.
    assert!(
        evs[0].detail.as_deref().unwrap().contains("NOT executed"),
        "invoke.agent must be recorded-not-executed in gate/observe"
    );
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

/// F1 (section 11.4): the environment classifier. `deny` matches take priority and
/// are case-insensitive; a non-empty `allow` denies when no allowed token is
/// present; an empty constraint never fires.
#[test]
fn env_violation_classifies_deny_and_allowlist() {
    use crate::snapshot::EnvConstraint;
    let cmd = |c: &str| Event {
        kind: EventKind::CommandRequested,
        session_id: None,
        file: None,
        language: None,
        content: None,
        line: None,
        command: Some(c.into()),
        env: Default::default(),
        files: vec![],
        loaded_skills: vec![],
        recorded_evidence: vec![],
    };
    let env = EnvConstraint {
        allow: vec!["local".into(), "docker-dev".into()],
        deny: vec!["staging".into(), "production".into()],
    };
    // deny wins, case-insensitive.
    assert!(env_violation(&env, &cmd("psql PRODUCTION-db")).is_some());
    // allowed environment passes.
    assert!(env_violation(&env, &cmd("psql local/app")).is_none());
    // no allowed token present → allowlist miss.
    assert!(env_violation(&env, &cmd("psql somewhere/app")).is_some());
    // whole-word: `reproduction` must NOT trigger the `production` deny.
    assert!(env_violation(&env, &cmd("psql reproduction-notes")).is_some()); // allowlist miss, not a deny
    assert!(
        !env_violation(&env, &cmd("psql reproduction-notes"))
            .unwrap()
            .contains("denied"),
        "`reproduction` must not match the `production` deny token"
    );
    // whole-word: `localstack` must NOT satisfy the `local` allowlist.
    assert!(
        env_violation(&env, &cmd("psql localstack-host/app")).is_some(),
        "`localstack` must not count as the allowed `local` environment"
    );
    // empty constraint never fires.
    let empty = EnvConstraint {
        allow: vec![],
        deny: vec![],
    };
    assert!(env_violation(&empty, &cmd("psql production-db")).is_none());
}

/// A rule with NO scope declared (or a scope that matches everything) is
/// meant to govern the WORKSPACE's own content — not arbitrary files a
/// client happens to edit outside the project (e.g. Claude Code's own
/// `~/.claude/plans/*.md` scratch file). Live bug: `require-red-before-
/// write` demanded RED test evidence for a Plan Mode file that isn't
/// production code and isn't even part of the governed project, because
/// nothing compared `event.file` against the workspace root at all.
#[test]
fn a_rule_without_scope_does_not_fire_on_a_file_outside_the_workspace() {
    let mut rule = blocking_rule();
    rule.scope = None; // matches everything BUT for the workspace-boundary check
    let s = snap(vec![rule]);
    let workspace = tempfile::tempdir().unwrap();
    let outside_file = workspace.path().parent().unwrap().join("elsewhere/x.dart");

    let evs = evaluate_event(
        &s,
        &edited(outside_file.to_str().unwrap(), "x // TODO"),
        workspace.path(),
        Mode::Enforce,
    );
    assert!(
        evs.is_empty(),
        "a file outside the workspace root must not evaluate any rule, got: {evs:?}"
    );
}

/// The same rule DOES fire normally for a file genuinely inside the
/// workspace, given as an absolute path (the real shape Claude Code sends).
#[test]
fn a_rule_without_scope_still_fires_inside_the_workspace_absolute_path() {
    let mut rule = blocking_rule();
    rule.scope = None;
    let s = snap(vec![rule]);
    let workspace = tempfile::tempdir().unwrap();
    let inside_file = workspace.path().join("lib/a.dart");

    let evs = evaluate_event(
        &s,
        &edited(inside_file.to_str().unwrap(), "x // TODO"),
        workspace.path(),
        Mode::Enforce,
    );
    assert_eq!(
        evs.len(),
        1,
        "an in-workspace absolute path must still fire"
    );
}

/// A RELATIVE `event.file` (the shape every existing test in this module
/// uses, e.g. `edited("lib/a.dart", ...)`) is workspace-relative by
/// definition and must keep firing exactly as before — the workspace-root
/// comparison only applies to absolute paths. `workspace_root` itself is
/// never canonicalized anywhere in this codebase (confirmed: no
/// `.canonicalize()` call exists for it), so it can legitimately be a
/// relative path like `.` too — this must not break that either.
#[test]
fn a_relative_file_path_is_unaffected_by_the_workspace_boundary_check() {
    let s = snap(vec![blocking_rule()]);
    let evs = evaluate_event(
        &s,
        &edited("lib/a.dart", "x // TODO"),
        Path::new("."),
        Mode::Enforce,
    );
    assert_eq!(evs.len(), 1);
}

fn branch(decision: Decision) -> CompiledBranch {
    CompiledBranch {
        decision,
        load_skills: vec![],
        load_capabilities: vec![],
        report_message: None,
        invoke_agent: None,
    }
}

/// A precondition-gated rule (like `require-audit-before-commit`): a precondition
/// with `onFail: block`, `enforcement.valid: allow`, and NO `validate`/`always`.
fn precondition_gate_rule() -> CompiledRule {
    CompiledRule {
        id: "require-audit".into(),
        version: Some("1.0.0".into()),
        author: "t".into(),
        adr_ref: "adr:ADR-1".into(),
        review_after: "P6M".into(),
        reversibility: Some(keel_core::Reversibility::Reversible),
        scope: None,
        on: vec![EventKind::CommandRequested],
        when: None,
        detect: None,
        preconditions: vec![CompiledPrecondition {
            using: CompiledToolRef::Builtin("evidence.recorded".into()),
            with: Some(serde_json::json!({"event": "task.completed", "verdict": "invalid"})),
            on_fail_declared: Decision::Block,
        }],
        validate: None,
        enforcement: CompiledEnforcement {
            invalid: None,
            unknown: None,
            valid: Some(branch(Decision::Allow)),
            always: None,
        },
        constraints: None,
        origin_layer: None,
        locked_at: None,
    }
}

fn commit_event(recorded: Vec<(EventKind, Verdict)>) -> Event {
    Event {
        kind: EventKind::CommandRequested,
        session_id: Some("s".into()),
        file: None,
        language: None,
        content: None,
        line: None,
        command: Some("git commit -m x".into()),
        env: Default::default(),
        files: vec![],
        loaded_skills: vec![],
        recorded_evidence: recorded,
    }
}

/// When a precondition-gated rule's precondition PASSES (the required evidence
/// exists) and it has no `validate`, the gate is satisfied → `Valid` → the
/// `valid`/allow branch. Regression guard for the bug where it fell to
/// `Unknown`/review and never allowed the action.
#[test]
fn precondition_gate_allows_when_precondition_passes() {
    let s = snap(vec![precondition_gate_rule()]);
    let ev = commit_event(vec![(EventKind::TaskCompleted, Verdict::Invalid)]);
    let evs = evaluate_event(&s, &ev, Path::new("/ws"), Mode::Enforce);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].verdict, Verdict::Valid);
    assert_eq!(evs[0].effective_decision, Decision::Allow);
}

/// The same rule still BLOCKS when the required evidence is absent (precondition
/// fails → `onFail` block).
#[test]
fn precondition_gate_blocks_when_precondition_fails() {
    let s = snap(vec![precondition_gate_rule()]);
    let ev = commit_event(vec![]);
    let evs = evaluate_event(&s, &ev, Path::new("/ws"), Mode::Enforce);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].verdict, Verdict::Invalid);
    assert_eq!(evs[0].effective_decision, Decision::Block);
}

/// Narrowing guard: a rule with NO gate at all (no preconditions, no validate,
/// no always) stays `Unknown` — genuinely undecidable, not silently allowed.
#[test]
fn ungated_rule_without_validate_stays_unknown() {
    let mut rule = precondition_gate_rule();
    rule.preconditions = vec![];
    let s = snap(vec![rule]);
    let ev = commit_event(vec![]);
    let evs = evaluate_event(&s, &ev, Path::new("/ws"), Mode::Enforce);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].verdict, Verdict::Unknown);
}
