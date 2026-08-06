// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `compile` (relocated out of src; included via #[path] in src/compile.rs).

use super::*;
use crate::workspace::WorkspaceFiles;
use keel_dsl::{Document, parse_documents};

fn files_from_yaml(yaml: &str) -> WorkspaceFiles {
    let mut files = WorkspaceFiles::empty(std::path::PathBuf::from("/tmp/ws"));
    for doc in parse_documents(yaml).unwrap() {
        match doc {
            Document::Rule(r) => files.rules.push(*r),
            Document::Tool(t) => files.tools.push(*t),
            Document::Skill(k) => files.skills.push(*k),
            Document::Agent(a) => files.agents.push(*a),
            Document::AgentExecutor(x) => files.executors.push(*x),
            Document::RuleTest(t) => files.tests.push(*t),
            Document::Workspace(_)
            | Document::RepositoryRegistry(_)
            | Document::Profile(_)
            | Document::Exception(_) => {}
        }
    }
    files
}

const IRREVERSIBLE_NO_UNKNOWN: &str = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: gate, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  reversibility: irreversible
  on: [command.requested]
  validate: { using: "tool:sql.classify" }
  enforcement:
    invalid: { decision: block }
    valid: { decision: allow }
---
apiVersion: keel/v1alpha1
kind: Tool
metadata: { id: sql.classify }
spec: { command: ["true"], output: exit-code }
"#;

/// Floor section 4.7: irreversible without an unknown branch → the compiler
/// creates it with deny-pending-approval. Uncertainty never goes without
/// a destination.
#[test]
fn irreversible_rule_gets_unknown_floor() {
    let files = files_from_yaml(IRREVERSIBLE_NO_UNKNOWN);
    let out = compile(&files, "2026-01-01T00:00:00Z".into()).unwrap();
    let rule = out.snapshot.rule_by_id("gate").unwrap();
    assert_eq!(
        rule.enforcement.unknown.as_ref().unwrap().decision,
        Decision::DenyPendingApproval
    );
}

/// Floor section 4.7: an unknown branch DECLARED below the floor is RAISED with
/// a warning (never silently).
#[test]
fn irreversible_unknown_below_floor_is_raised_with_warning() {
    let yaml = IRREVERSIBLE_NO_UNKNOWN.replace(
        "    invalid: { decision: block }",
        "    invalid: { decision: block }\n    unknown: { decision: review }",
    );
    let files = files_from_yaml(&yaml);
    let out = compile(&files, "2026-01-01T00:00:00Z".into()).unwrap();
    let rule = out.snapshot.rule_by_id("gate").unwrap();
    assert_eq!(
        rule.enforcement.unknown.as_ref().unwrap().decision,
        Decision::DenyPendingApproval
    );
    assert!(out.warnings.iter().any(|w| w.contains("floor section 4.7")));
}

#[test]
fn unresolved_tool_reference_is_a_compile_error() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r1, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  on: [file.edited]
  validate: { using: "tool:ghost.tool" }
  enforcement:
    invalid: { decision: block }
"#;
    let files = files_from_yaml(yaml);
    let err = compile(&files, "t".into()).unwrap_err();
    assert!(matches!(err, CompileError::UnresolvedTool { .. }));
}

#[test]
fn duplicate_rule_ids_conflict_loudly() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: dup, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  on: [file.edited]
  validate: { using: "builtin:text.contains", with: { value: "x" } }
  enforcement: { invalid: { decision: review } }
---
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: dup, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  on: [file.edited]
  validate: { using: "builtin:text.contains", with: { value: "y" } }
  enforcement: { invalid: { decision: review } }
"#;
    let files = files_from_yaml(yaml);
    assert!(matches!(
        compile(&files, "t".into()).unwrap_err(),
        CompileError::DuplicateId(_)
    ));
}

#[test]
fn invalid_regex_fails_at_compile_not_runtime() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r1, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  on: [file.edited]
  detect: { using: "builtin:text.regex", with: { pattern: "([unclosed" } }
  validate: { using: "builtin:text.contains", with: { value: "x" } }
  enforcement: { invalid: { decision: review } }
"#;
    let files = files_from_yaml(yaml);
    assert!(matches!(
        compile(&files, "t".into()).unwrap_err(),
        CompileError::InvalidRegex { .. }
    ));
}

/// Defense in depth: the schema catches the grossly invalid ("6 months"
/// does not even start with P); the compiler catches what LOOKS like
/// ISO-8601 but is not ("P6X" passes the schema's `^P` pattern).
#[test]
fn bad_review_after_fails_compile() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r1, author: t, adrRef: "adr:ADR-1", reviewAfter: "P6X" }
spec:
  on: [file.edited]
  validate: { using: "builtin:text.contains", with: { value: "x" } }
  enforcement: { invalid: { decision: review } }
"#;
    let files = files_from_yaml(yaml);
    assert!(matches!(
        compile(&files, "t".into()).unwrap_err(),
        CompileError::BadReviewAfter { .. }
    ));
}

/// F1 (section 11.4): a malformed `constraints` shape is a BUILD error, never a
/// silently-ignored field at eval — `allow` must be a list of strings.
#[test]
fn malformed_constraints_fails_compile() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r1, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  on: [command.requested]
  validate: { using: "builtin:text.contains", with: { value: "x" } }
  enforcement: { invalid: { decision: review } }
  constraints: { environment: { allow: "not-a-list" } }
"#;
    let files = files_from_yaml(yaml);
    assert!(matches!(
        compile(&files, "t".into()).unwrap_err(),
        CompileError::BadConstraints { .. }
    ));
}

/// Mismo workspace compilado dos veces → mismo hash (invariante 9).
#[test]
fn same_workspace_same_hash() {
    let files = files_from_yaml(IRREVERSIBLE_NO_UNKNOWN);
    let a = compile(&files, "2026-01-01T00:00:00Z".into()).unwrap();
    let b = compile(&files, "2026-12-31T23:59:59Z".into()).unwrap();
    assert_eq!(a.snapshot.hash, b.snapshot.hash);
}
