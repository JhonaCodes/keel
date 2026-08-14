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
            Document::RuleTest(t) => files.tests.push(*t),
            Document::Containment(c) => files.containments.push(*c),
            Document::Workspace(_)
            | Document::RepositoryRegistry(_)
            | Document::Profile(_)
            | Document::Exception(_)
            | Document::Knowledge(_)
            | Document::Workflow(_)
            | Document::Contract(_)
            | Document::Hook(_)
            | Document::MCPProvider(_)
            | Document::ModelExecutor(_)
            | Document::AgentRoutingPolicy(_)
            | Document::Policy(_) => {}
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
fn evidence_recorded_precondition_compiles() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r-evidence, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  on: [command.requested]
  preconditions:
    - using: "builtin:evidence.recorded"
      with: { event: "test.completed", verdict: invalid }
      onFail: block
  enforcement:
    valid: { decision: allow }
"#;
    let files = files_from_yaml(yaml);
    let out = compile(&files, "t".into()).unwrap();
    assert!(out.snapshot.rule_by_id("r-evidence").is_some());
}

/// Regression: an invented builtin precondition must still fail compilation
/// — `evidence.recorded` was ADDED to `BUILTIN_PRECONDITIONS`, not a
/// loosening of the allowlist itself.
#[test]
fn unknown_builtin_precondition_is_still_a_compile_error() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r-ghost, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  on: [command.requested]
  preconditions:
    - using: "builtin:not.a.real.precondition"
      onFail: block
  enforcement:
    valid: { decision: allow }
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

const SKILL_BAD_NAME: &str = r#"
apiVersion: keel/v1alpha1
kind: Skill
metadata: { id: access-patterns, version: 0.1.0 }
spec: { compact: global/skills/access.md }
"#;

const SKILL_KEEL_NAME: &str = r#"
apiVersion: keel/v1alpha1
kind: Skill
metadata: { id: access-patterns, version: 0.1.0 }
spec: { compact: global/skills/access_keel.md }
"#;

/// A skill's content file MUST end in `_keel.md` — enforced at compile so the
/// provenance suffix is a rule, not a convention.
#[test]
fn a_skill_content_file_must_end_in_keel_md() {
    let bad = files_from_yaml(SKILL_BAD_NAME);
    assert!(matches!(
        compile(&bad, "t".into()).unwrap_err(),
        CompileError::SkillNaming { .. }
    ));

    // The same skill with the `_keel.md` suffix compiles (a missing file is
    // only a warning, not an error).
    let ok = files_from_yaml(SKILL_KEEL_NAME);
    assert!(compile(&ok, "t".into()).is_ok());
}

#[test]
fn a_skill_can_embed_compact_and_full_content_inline() {
    let files = files_from_yaml(
        r#"
apiVersion: keel/v1alpha1
kind: Skill
metadata: { id: inline-guide, version: 0.1.0 }
spec:
  compact: |
    Use this short guide.
  full: |
    Use this full guide.
"#,
    );
    let snap = compile(&files, "t".into()).unwrap().snapshot;
    let skill = snap.skills.get("inline-guide").unwrap();

    assert_eq!(skill.compact, "<inline>");
    assert_eq!(
        skill.compact_content.as_deref(),
        Some("Use this short guide.")
    );
    assert_eq!(skill.full, None);
    assert_eq!(skill.full_content.as_deref(), Some("Use this full guide."));
}

/// The optional `description` (for the exposed catalog, D-013) flows through to
/// the compiled skill in the snapshot.
#[test]
fn a_skill_description_reaches_the_snapshot() {
    let with_desc = files_from_yaml(
        r#"
apiVersion: keel/v1alpha1
kind: Skill
metadata: { id: verify, version: 0.1.0 }
spec: { description: "Gate de verificacion antes de cerrar", compact: global/skills/verify_keel.md }
"#,
    );
    let snap = compile(&with_desc, "t".into()).unwrap().snapshot;
    assert_eq!(
        snap.skills.get("verify").unwrap().description.as_deref(),
        Some("Gate de verificacion antes de cerrar")
    );

    // Absent description → None (backward compatible).
    let without = files_from_yaml(SKILL_KEEL_NAME);
    let snap2 = compile(&without, "t".into()).unwrap().snapshot;
    assert_eq!(
        snap2.skills.get("access-patterns").unwrap().description,
        None
    );
}

// ── Declarative routing derivation (D-014) ──────────────────────────────────

#[test]
fn compile_match_infers_context_cue_from_derived_terms() {
    // No authored match: a `pr` term deterministically implies the github_pr
    // structured object type (CONTEXT_CUES), the highest-weight signal.
    let m = compile_match(None, "keel_review_pr", Some("review a pull request"));
    assert!(m.context.contains(&"github_pr".to_string()));
    assert!(
        m.terms.is_empty(),
        "no authored terms → explicit terms empty"
    );
    assert!(!m.autoload);
}

#[test]
fn compile_match_preserves_authored_terms_context_and_autoload() {
    let authored = keel_dsl::MatchSpec {
        terms: vec!["coderabbit".to_string()],
        context: vec!["github_pr".to_string()],
        autoload: true,
    };
    let m = compile_match(Some(&authored), "keel_review_pr", Some("review a PR"));
    assert_eq!(m.terms, vec!["coderabbit".to_string()]);
    assert!(m.autoload);
    // Authored github_pr is not duplicated by the derived `pr` cue.
    assert_eq!(
        m.context.iter().filter(|c| *c == "github_pr").count(),
        1,
        "an authored context must not be duplicated by a derived cue"
    );
}

#[test]
fn sibling_review_skills_disambiguate_by_explicit_terms() {
    // The exact concern: two review-pr skills. Only the CodeRabbit one declares
    // `coderabbit`, so only its compiled match carries that high-weight term —
    // the deterministic lever the native router ranks on.
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Skill
metadata: { id: keel_review_pr_coderabbit, version: 0.1.0 }
spec:
  description: "Resolve CodeRabbit review comments on a PR"
  match: { terms: [coderabbit], context: [github_pr] }
  compact: global/skills/keel_review_pr_coderabbit_keel.md
---
apiVersion: keel/v1alpha1
kind: Skill
metadata: { id: keel_review_pr_team, version: 0.1.0 }
spec:
  description: "Team review of a PR"
  compact: global/skills/keel_review_pr_team_keel.md
"#;
    let snap = compile(&files_from_yaml(yaml), "t".into())
        .unwrap()
        .snapshot;
    let cr = &snap.skills.get("keel_review_pr_coderabbit").unwrap().match_;
    let team = &snap.skills.get("keel_review_pr_team").unwrap().match_;
    assert_eq!(cr.terms, vec!["coderabbit".to_string()]);
    assert!(
        team.terms.is_empty(),
        "the team skill declares no explicit term, so it cannot win a coderabbit prompt"
    );
    // Both still carry the github_pr context (declared / derived), so both
    // remain candidates for a PR prompt — disambiguation is by the extra term.
    assert!(cr.context.contains(&"github_pr".to_string()));
    assert!(team.context.contains(&"github_pr".to_string()));
}
