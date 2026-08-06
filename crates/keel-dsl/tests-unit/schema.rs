// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `schema` (relocated out of src; included via #[path] in src/schema.rs).

use super::*;

/// ADR-023: a Rule without author/adrRef/reviewAfter does NOT compile.
#[test]
fn rule_without_provenance_is_rejected() {
    let raw: serde_json::Value = serde_json::from_str(
        r#"{
              "apiVersion": "keel/v1alpha1",
              "kind": "Rule",
              "metadata": { "id": "x" },
              "spec": { "on": ["file.edited"], "enforcement": { "valid": { "decision": "allow" } } }
            }"#,
    )
    .unwrap();
    let err = validate("Rule", &raw).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("author"), "must require author: {msg}");
    assert!(msg.contains("adrRef"), "must require adrRef: {msg}");
    assert!(
        msg.contains("reviewAfter"),
        "must require reviewAfter: {msg}"
    );
}

/// Unknown fields are rejected: this is how Phase 0a detects that a real
/// gate uses vocabulary the DSL does not yet express (instead of
/// swallowing it silently — the failure mode Keel fights).
#[test]
fn unknown_fields_are_rejected_not_swallowed() {
    let raw: serde_json::Value = serde_json::from_str(
        r#"{
              "apiVersion": "keel/v1alpha1",
              "kind": "Rule",
              "metadata": { "id": "x", "author": "a", "adrRef": "adr:ADR-1", "reviewAfter": "P6M" },
              "spec": {
                "on": ["file.edited"],
                "enforcement": { "valid": { "decision": "allow" } },
                "madeUpField": true
              }
            }"#,
    )
    .unwrap();
    assert!(validate("Rule", &raw).is_err());
}
