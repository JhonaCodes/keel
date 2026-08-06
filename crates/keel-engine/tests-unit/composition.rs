// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `composition` (relocated out of src; included via #[path]).
//!
//! Exercised through the real `compile_layered` path: a `global` layer plus a
//! `project` layer with the same rule id. A `locked` global rule may only be
//! STRENGTHENED by the project; any weakening is a monotonicity error naming
//! the dimension (spec section 7.4). `overridable`, no-lock and `merge: append`
//! are covered too.

use super::Dimension;
use crate::compile::{CompileError, CompileLayer, CompileOutcome, compile_layered};
use crate::composition::MonotonicityViolation;
use crate::snapshot::CompiledRule;
use crate::workspace::WorkspaceFiles;
use keel_core::Decision;
use keel_dsl::{Document, parse_documents};
use std::path::PathBuf;

fn ws(yaml: &str) -> WorkspaceFiles {
    let mut f = WorkspaceFiles::empty(PathBuf::from("/tmp/keel-compose-it"));
    for doc in parse_documents(yaml).expect("layer yaml parses") {
        match doc {
            Document::Rule(r) => f.rules.push(*r),
            Document::Tool(t) => f.tools.push(*t),
            Document::Skill(s) => f.skills.push(*s),
            Document::Agent(a) => f.agents.push(*a),
            Document::AgentExecutor(x) => f.executors.push(*x),
            _ => {}
        }
    }
    f
}

fn compile_chain(
    global: &WorkspaceFiles,
    project: &WorkspaceFiles,
) -> Result<CompileOutcome, CompileError> {
    compile_layered(
        &[
            CompileLayer {
                label: "global".to_string(),
                files: global,
            },
            CompileLayer {
                label: "project:demo".to_string(),
                files: project,
            },
        ],
        "t".to_string(),
    )
}

fn violation(res: Result<CompileOutcome, CompileError>) -> MonotonicityViolation {
    match res {
        Err(CompileError::Monotonicity(v)) => v,
        other => panic!("expected a monotonicity violation, got {other:?}"),
    }
}

fn rule_of<'a>(outcome: &'a CompileOutcome, id: &str) -> &'a CompiledRule {
    outcome
        .snapshot
        .rules
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("rule `{id}` missing from snapshot"))
}

// A locked global rule the project tries to weaken along one dimension.
const LOCKED_BLOCK_ON_SRC: &str = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: sec.no-raw, author: platform, adrRef: adr:ADR-1, reviewAfter: P6M }
spec:
  locked: true
  scope: { paths: { include: ["src/**"] } }
  on: [file.edited]
  validate: { using: builtin:text.regex, with: { pattern: "rawQuery" } }
  enforcement:
    invalid: { decision: block, report: { message: "use the builder" } }
    valid: { decision: allow }
"#;

#[test]
fn project_strengthening_a_locked_rule_composes() {
    let global = ws(LOCKED_BLOCK_ON_SRC);
    // Same decision, wider coverage (adds a language), extra skill — all ≥.
    let project = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: sec.no-raw, author: team, adrRef: adr:ADR-2, reviewAfter: P6M }
spec:
  scope: { paths: { include: ["src/**"] } }
  on: [file.edited]
  validate: { using: builtin:text.regex, with: { pattern: "rawQuery" } }
  enforcement:
    invalid: { decision: block, report: { message: "use the builder" } }
    valid: { decision: allow }
"#);
    let outcome = compile_chain(&global, &project).expect("strengthening composes");
    assert_eq!(
        rule_of(&outcome, "sec.no-raw")
            .enforcement
            .invalid
            .as_ref()
            .unwrap()
            .decision,
        Decision::Block
    );
}

#[test]
fn downgrading_a_locked_decision_is_a_d3_violation() {
    let global = ws(LOCKED_BLOCK_ON_SRC);
    let project = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: sec.no-raw, author: team, adrRef: adr:ADR-2, reviewAfter: P6M }
spec:
  scope: { paths: { include: ["src/**"] } }
  on: [file.edited]
  validate: { using: builtin:text.regex, with: { pattern: "rawQuery" } }
  enforcement:
    invalid: { decision: review }
    valid: { decision: allow }
"#);
    let v = violation(compile_chain(&global, &project));
    assert_eq!(v.dimension, Dimension::Consequence);
    assert_eq!(v.locked_at, "global");
    assert_eq!(v.violated_by, "project:demo");
}

#[test]
fn adding_an_exclude_is_a_d1_violation() {
    let global = ws(LOCKED_BLOCK_ON_SRC);
    let project = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: sec.no-raw, author: team, adrRef: adr:ADR-2, reviewAfter: P6M }
spec:
  scope: { paths: { include: ["src/**"], exclude: ["src/reports/**"] } }
  on: [file.edited]
  validate: { using: builtin:text.regex, with: { pattern: "rawQuery" } }
  enforcement:
    invalid: { decision: block }
    valid: { decision: allow }
"#);
    let v = violation(compile_chain(&global, &project));
    assert_eq!(v.dimension, Dimension::Coverage);
}

#[test]
fn narrowing_languages_is_a_d1_violation() {
    // Global governs ALL languages (no languages listed); project restricts.
    let global = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: p, adrRef: adr:ADR-1, reviewAfter: P6M }
spec:
  locked: true
  on: [file.edited]
  enforcement: { invalid: { decision: block }, valid: { decision: allow } }
"#);
    let project = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: t, adrRef: adr:ADR-2, reviewAfter: P6M }
spec:
  scope: { languages: ["dart"] }
  on: [file.edited]
  enforcement: { invalid: { decision: block }, valid: { decision: allow } }
"#);
    assert_eq!(
        violation(compile_chain(&global, &project)).dimension,
        Dimension::Coverage
    );
}

#[test]
fn substituting_the_validator_is_a_d2_violation() {
    let global = ws(LOCKED_BLOCK_ON_SRC);
    let project = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: sec.no-raw, author: team, adrRef: adr:ADR-2, reviewAfter: P6M }
spec:
  scope: { paths: { include: ["src/**"] } }
  on: [file.edited]
  validate: { using: builtin:text.regex, with: { pattern: "somethingElse" } }
  enforcement:
    invalid: { decision: block }
    valid: { decision: allow }
"#);
    assert_eq!(
        violation(compile_chain(&global, &project)).dimension,
        Dimension::Sensitivity
    );
}

#[test]
fn dropping_a_loaded_skill_is_a_d4_violation() {
    let global = ws(r#"
apiVersion: keel/v1alpha1
kind: Skill
metadata: { id: guide }
spec: { compact: guide.md }
---
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: p, adrRef: adr:ADR-1, reviewAfter: P6M }
spec:
  locked: true
  on: [file.edited]
  enforcement:
    invalid: { decision: block, load: { skills: ["skill:guide"] } }
    valid: { decision: allow }
"#);
    // Project keeps decision + scope but drops the loaded skill.
    let project = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: t, adrRef: adr:ADR-2, reviewAfter: P6M }
spec:
  on: [file.edited]
  enforcement:
    invalid: { decision: block }
    valid: { decision: allow }
"#);
    assert_eq!(
        violation(compile_chain(&global, &project)).dimension,
        Dimension::Load
    );
}

#[test]
fn overridable_lets_the_project_replace_even_weaker() {
    let global = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: p, adrRef: adr:ADR-1, reviewAfter: P6M }
spec:
  locked: true
  overridable: true
  on: [file.edited]
  enforcement: { invalid: { decision: block }, valid: { decision: allow } }
"#);
    let project = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: t, adrRef: adr:ADR-2, reviewAfter: P6M }
spec:
  on: [file.edited]
  enforcement: { invalid: { decision: review }, valid: { decision: allow } }
"#);
    let outcome = compile_chain(&global, &project).expect("overridable permits replacement");
    assert_eq!(
        rule_of(&outcome, "r")
            .enforcement
            .invalid
            .as_ref()
            .unwrap()
            .decision,
        Decision::Review
    );
}

#[test]
fn without_a_lock_the_project_replaces_freely() {
    let global = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: p, adrRef: adr:ADR-1, reviewAfter: P6M }
spec:
  on: [file.edited]
  enforcement: { invalid: { decision: block }, valid: { decision: allow } }
"#);
    let project = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: t, adrRef: adr:ADR-2, reviewAfter: P6M }
spec:
  on: [file.edited]
  enforcement: { invalid: { decision: review }, valid: { decision: allow } }
"#);
    let outcome = compile_chain(&global, &project).expect("no lock → free replace");
    assert_eq!(
        rule_of(&outcome, "r")
            .enforcement
            .invalid
            .as_ref()
            .unwrap()
            .decision,
        Decision::Review
    );
}

#[test]
fn merge_append_escalates_decision_and_adds_a_validator() {
    let global = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: p, adrRef: adr:ADR-1, reviewAfter: P6M }
spec:
  locked: true
  on: [file.edited]
  enforcement: { invalid: { decision: review }, valid: { decision: allow } }
"#);
    // Appends a validator the base lacked and escalates the decision.
    let project = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: t, adrRef: adr:ADR-2, reviewAfter: P6M }
spec:
  merge: append
  on: [file.edited]
  validate: { using: builtin:text.contains, with: { text: "TODO" } }
  enforcement: { invalid: { decision: block }, valid: { decision: allow } }
"#);
    let outcome = compile_chain(&global, &project).expect("append composes");
    let r = rule_of(&outcome, "r");
    assert_eq!(
        r.enforcement.invalid.as_ref().unwrap().decision,
        Decision::Block,
        "decision escalated"
    );
    assert!(r.validate.is_some(), "validator added by append");
}

#[test]
fn merge_append_cannot_substitute_a_validator() {
    let global = ws(LOCKED_BLOCK_ON_SRC);
    let project = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: sec.no-raw, author: t, adrRef: adr:ADR-2, reviewAfter: P6M }
spec:
  merge: append
  on: [file.edited]
  validate: { using: builtin:text.regex, with: { pattern: "different" } }
  enforcement: { invalid: { decision: block }, valid: { decision: allow } }
"#);
    assert_eq!(
        violation(compile_chain(&global, &project)).dimension,
        Dimension::Sensitivity
    );
}

#[test]
fn duplicate_rule_id_within_one_layer_is_a_conflict() {
    let dup = ws(r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: p, adrRef: adr:ADR-1, reviewAfter: P6M }
spec: { on: [file.edited], enforcement: { valid: { decision: allow } } }
---
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r, author: p, adrRef: adr:ADR-1, reviewAfter: P6M }
spec: { on: [file.edited], enforcement: { valid: { decision: allow } } }
"#);
    let empty = WorkspaceFiles::empty(PathBuf::from("/tmp/x"));
    assert!(matches!(
        compile_chain(&dup, &empty),
        Err(CompileError::DuplicateId(_))
    ));
}
