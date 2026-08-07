// SPDX-License-Identifier: Apache-2.0
//! Phase-1 composition vocabulary (spec section 7.3 + core kinds section 11.1):
//! the DSL must express `locked`/`overridable`/`merge` on a rule and the kinds
//! `RepositoryRegistry`, `Profile` and `Exception` WITHOUT LOSS. This is the
//! Phase-0a expressiveness criterion applied to the composition model: the
//! vocabulary is proven here before any composition runtime is written.

use keel_dsl::{Document, Merge, parse_documents};

fn roundtrip_stable(doc: &Document) {
    let id = doc.metadata().id.clone();
    let v1 = serde_json::to_value(doc).unwrap_or_else(|e| panic!("`{id}` serialize: {e}"));
    let reparsed: Document =
        serde_json::from_value(v1.clone()).unwrap_or_else(|e| panic!("`{id}` re-parse: {e}"));
    let v2 = serde_json::to_value(&reparsed).unwrap();
    assert_eq!(v1, v2, "unstable round-trip in `{id}`: something is lost");
}

#[test]
fn rule_inheritance_fields_parse_and_roundtrip() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata:
  id: org.no-raw-queries
  author: platform
  adrRef: adr:ADR-031
  reviewAfter: P6M
spec:
  reversibility: reversible
  locked: true
  merge: append
  on: [file.edited]
  enforcement:
    invalid: { decision: block }
    valid: { decision: allow }
"#;
    let docs = parse_documents(yaml).expect("rule with inheritance keys must parse");
    let Document::Rule(rule) = &docs[0] else {
        panic!("expected a Rule");
    };
    assert!(rule.spec.locked, "locked must be preserved");
    assert!(!rule.spec.overridable, "overridable defaults to false");
    assert_eq!(
        rule.spec.merge,
        Some(Merge::Append),
        "merge:append preserved"
    );
    roundtrip_stable(&docs[0]);
}

#[test]
fn rule_without_inheritance_keys_defaults_to_plain() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata:
  id: project.plain
  author: me
  adrRef: adr:ADR-001
  reviewAfter: P6M
spec:
  on: [file.edited]
  enforcement:
    valid: { decision: allow }
"#;
    let docs = parse_documents(yaml).unwrap();
    let Document::Rule(rule) = &docs[0] else {
        panic!("expected a Rule");
    };
    assert!(!rule.spec.locked);
    assert!(!rule.spec.overridable);
    assert_eq!(rule.spec.merge, None);
    // A plain rule serializes WITHOUT the flags (off round-trips as absent).
    let v = serde_json::to_value(&docs[0]).unwrap();
    assert!(v["spec"].get("locked").is_none(), "off flag stays absent");
    assert!(v["spec"].get("merge").is_none());
}

#[test]
fn merge_rejects_unknown_strategy() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: bad.merge, author: me, adrRef: adr:ADR-001, reviewAfter: P6M }
spec:
  on: [file.edited]
  merge: replace
  enforcement: { valid: { decision: allow } }
"#;
    assert!(
        parse_documents(yaml).is_err(),
        "merge only accepts `append` (section 7.3)"
    );
}

#[test]
fn repository_registry_parses_identity_mapping() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: RepositoryRegistry
metadata: { id: nui-repositories }
spec:
  repositories:
    - provider: github
      id: NuiMarkets/con-app
      project: project:nui/con-app
      locked: true
"#;
    let docs = parse_documents(yaml).expect("RepositoryRegistry must parse");
    let Document::RepositoryRegistry(reg) = &docs[0] else {
        panic!("expected a RepositoryRegistry");
    };
    let entry = &reg.spec.repositories[0];
    assert_eq!(entry.provider, "github");
    assert_eq!(entry.project, "project:nui/con-app");
    assert!(entry.locked);
    roundtrip_stable(&docs[0]);
}

#[test]
fn profile_parses_preferences() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Profile
metadata: { id: jhonatan }
spec:
  client: codex
  preferences:
    implementationStrategy: tdd
    verbosity: compact
"#;
    let docs = parse_documents(yaml).expect("Profile must parse");
    let Document::Profile(profile) = &docs[0] else {
        panic!("expected a Profile");
    };
    assert_eq!(profile.spec.client.as_deref(), Some("codex"));
    assert_eq!(
        profile.spec.preferences.as_ref().unwrap()["implementationStrategy"],
        "tdd"
    );
    roundtrip_stable(&docs[0]);
}

#[test]
fn exception_parses_owner_scope_and_expiry() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Exception
metadata: { id: reports-legacy-queries }
spec:
  rule: rule:org/nui/security.no-raw-queries
  owner: organization:nui
  reason: "Legacy reporting module migrates next quarter."
  scope:
    paths:
      include: ["src/Reports/**"]
  expiry: "2026-12-31"
"#;
    let docs = parse_documents(yaml).expect("Exception must parse");
    let Document::Exception(exc) = &docs[0] else {
        panic!("expected an Exception");
    };
    assert_eq!(exc.spec.rule, "rule:org/nui/security.no-raw-queries");
    assert_eq!(exc.spec.owner, "organization:nui");
    assert_eq!(exc.spec.expiry, "2026-12-31");
    assert_eq!(
        exc.spec.scope.paths.as_ref().unwrap().include,
        vec!["src/Reports/**".to_string()]
    );
    roundtrip_stable(&docs[0]);
}

#[test]
fn exception_requires_owner_and_expiry() {
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Exception
metadata: { id: incomplete }
spec:
  rule: rule:org/x
  reason: "missing owner and expiry"
"#;
    assert!(
        parse_documents(yaml).is_err(),
        "an Exception without owner/expiry is not governed (section 7.4)"
    );
}

#[test]
fn exception_without_scope_is_rejected() {
    // section 7.4: a governed exception has a BOUNDED scope. A scope-less
    // exception would be an unbounded weakening of a locked rule — forbidden.
    let yaml = r#"
apiVersion: keel/v1alpha1
kind: Exception
metadata: { id: unbounded }
spec:
  rule: rule:org/x
  owner: organization:nui
  reason: "no bounded scope — must be rejected"
  expiry: "2026-12-31"
"#;
    assert!(
        parse_documents(yaml).is_err(),
        "an unbounded (scope-less) exception must not be authorable"
    );
}
