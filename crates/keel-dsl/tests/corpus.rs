// SPDX-License-Identifier: Apache-2.0
//! Corpus test — Phase 0a (spec section 15.1).
//!
//! Loads ALL the rules from spec section 11.3–11.5 and asserts:
//!   1. they pass the JSON Schema (with ADR-023 active),
//!   2. the `parse → serialize → parse` round-trip is stable — nothing is lost.
//!
//! If the DSL cannot represent a rule from the spec itself, this test fails:
//! it is the mechanical proof of expressiveness, the passing criterion of 0a.

use keel_dsl::{Document, parse_documents};

const CORPUS: &str = include_str!("corpus/rules_11_4.yaml");

#[test]
fn corpus_11_4_parses_completely() {
    let docs =
        parse_documents(CORPUS).expect("the section 11.3–11.5 corpus must parse without loss");
    // 9 rules: section 11.3 (1) + section 11.4 (7) + section 11.5 (1).
    assert_eq!(docs.len(), 9, "the full corpus must be present");
    for doc in &docs {
        assert!(matches!(doc, Document::Rule(_)), "the corpus is Rules only");
    }
}

#[test]
fn corpus_roundtrip_is_stable() {
    let docs = parse_documents(CORPUS).unwrap();
    for doc in docs {
        let id = doc.metadata().id.clone();
        // parse → canonical JSON → parse → canonical JSON: identical.
        let v1 = serde_json::to_value(&doc)
            .unwrap_or_else(|e| panic!("rule `{id}` does not serialize: {e}"));
        let reparsed: Document = serde_json::from_value(v1.clone())
            .unwrap_or_else(|e| panic!("rule `{id}` does not re-parse: {e}"));
        let v2 = serde_json::to_value(&reparsed).unwrap();
        assert_eq!(v1, v2, "unstable round-trip in `{id}`: something is lost");
    }
}

/// The fine-grained corpus details most easily "lost" in a poor DSL:
/// preconditions with onFail, environment constraints, invoke on unknown,
/// when.files.touch. Their presence is asserted explicitly.
#[test]
fn corpus_preserves_the_hard_parts() {
    let docs = parse_documents(CORPUS).unwrap();
    let rule = |id: &str| -> keel_dsl::RuleSpec {
        docs.iter()
            .find_map(|d| match d {
                Document::Rule(r) if r.metadata.id == id => Some(r.spec.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("rule `{id}` missing from corpus"))
    };

    // ADR-022: the three preconditions of the prod-write-gate, in order.
    let gate = rule("db.prod-write-gate");
    assert_eq!(gate.preconditions.len(), 3);
    assert_eq!(
        gate.preconditions[0].using.to_string(),
        "builtin:env.present"
    );
    assert_eq!(
        gate.preconditions[2].using.to_string(),
        "tool:awsume.session-active"
    );

    // Environment constraints of the SQL gate (section 11.4): preserved, not swallowed.
    let sql = rule("db.gate-sql-execution");
    let env = sql.constraints.expect("constraints must be preserved");
    assert_eq!(env["environment"]["deny"][1], "production");

    // Invoke on the unknown branch (section 11.3): it is parsed (and in Phase 0 only
    // recorded — never executed).
    let rn = rule("reactive-notifier.no-direct-data");
    let unknown = rn.enforcement.unknown.expect("unknown branch present");
    assert_eq!(
        unknown.invoke.expect("invoke present").agent,
        "agent:reactive-notifier.state-auditor"
    );

    // when.files.touch of the cognitive activation.
    let cog = rule("analysis.load-state-context");
    let when = cog.when.expect("when present");
    assert_eq!(
        when.any[0].files_touch.as_deref(),
        Some(&["lib/**/state/**".to_string()][..])
    );
    assert!(cog.enforcement.always.is_some());
}
