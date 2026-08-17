// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `routing` (relocated out of src; included via #[path]).

use super::*;
use crate::snapshot::{CompiledMatch, CompiledSkill, Snapshot};
use std::collections::{BTreeMap, BTreeSet};

fn words(text: &str) -> BTreeSet<String> {
    derive_terms(&[text]).into_iter().collect()
}

#[test]
fn derive_terms_splits_id_drops_stopwords_and_dedupes() {
    // The id splits on `_`; the `keel` provenance prefix is dropped as
    // structural noise; domain words (`review`, `pr`, `coderabbit`) survive; a
    // repeated word appears once.
    let terms = derive_terms(&["keel_review_pr_coderabbit", "review a PR with CodeRabbit"]);
    assert!(terms.contains(&"coderabbit".to_string()));
    assert!(
        terms.contains(&"pr".to_string()),
        "2-char domain keyword survives"
    );
    assert!(
        terms.contains(&"review".to_string()),
        "review is a domain keyword, not structural noise"
    );
    assert!(
        !terms.contains(&"keel".to_string()),
        "the keel provenance prefix must not pollute routing terms"
    );
    assert_eq!(
        terms.iter().filter(|t| *t == "coderabbit").count(),
        1,
        "terms must be deduped"
    );
}

#[test]
fn detect_contexts_reads_urls_and_cue_words() {
    let ctx = detect_contexts("please review https://github.com/o/r/pull/42 for coderabbit");
    assert!(ctx.contains("github_pr"), "a /pull/ URL implies github_pr");

    let ctx2 = detect_contexts("mira este ticket de linear.app/issue/ABC-1");
    assert!(ctx2.contains("linear_ticket"), "linear.app + ticket cue");

    assert!(
        detect_contexts("refactor the parser").is_empty(),
        "no structured cue → no context"
    );
}

#[test]
fn detect_contexts_reads_the_technology_off_a_code_moment() {
    // The moment text for a file edit is the file's content or its path. A
    // Flutter source resolves the framework straight off its imports, which is
    // what lets a `context: [platform/flutter]` block score at all.
    let ctx = detect_contexts(
        "import 'package:flutter/material.dart';\nclass X extends StatelessWidget {}",
    );
    assert!(
        ctx.contains("platform/flutter"),
        "flutter import → framework"
    );
    assert!(ctx.contains("platform/dart"), "and the language layer");

    let ctx2 = detect_contexts("lib/features/orders/order_view_model.dart");
    assert!(
        ctx2.contains("platform/dart"),
        "a bare .dart path is the language, NOT the framework: {ctx2:?}"
    );
    assert!(!ctx2.contains("platform/flutter"));

    let ctx3 = detect_contexts("crates/keel-engine/Cargo.toml");
    assert!(ctx3.contains("platform/rust"));
}

#[test]
fn ambiguous_short_tokens_do_not_leak_a_platform() {
    // `logger_rs` is a Dart package: `rs` must never imply Rust, or every Dart
    // file using it would route the Rust layer.
    let ctx = detect_contexts("import 'package:logger_rs/logger_rs.dart';");
    assert!(!ctx.contains("platform/rust"), "got {ctx:?}");
}

#[test]
fn score_weights_context_above_term_above_derived() {
    let prompt = words("review this pr for coderabbit");
    let ctx = detect_contexts("review this pr for coderabbit"); // → github_pr

    let full = CompiledMatch {
        terms: vec!["coderabbit".into()],
        derived: vec!["review".into(), "pr".into()],
        context: vec!["github_pr".into()],
        autoload: false,
    };
    let (total, trigger) = score(&prompt, &ctx, &full).expect("matches");
    // context(3) + term(2) + derived review(1) + derived pr(1) = 7.
    assert_eq!(total, 7);
    assert_eq!(
        trigger, "term:coderabbit",
        "label prefers the authored term (the disambiguator), not the shared context"
    );

    // A skill with only the derived overlap scores far lower — the ranking
    // separation that fixes the sibling-collision problem.
    let weak = CompiledMatch {
        derived: vec!["review".into(), "pr".into()],
        context: vec!["github_pr".into()],
        ..Default::default()
    };
    let (weak_total, _) = score(&prompt, &ctx, &weak).expect("matches");
    assert!(weak_total < total, "no explicit term → lower score");
}

fn skill(id: &str, terms: &[&str], context: &[&str], autoload: bool) -> CompiledSkill {
    CompiledSkill {
        id: id.to_string(),
        version: "0.1.0".into(),
        description: None,
        match_: CompiledMatch {
            terms: terms.iter().map(|s| s.to_string()).collect(),
            derived: derive_terms(&[id]),
            context: context.iter().map(|s| s.to_string()).collect(),
            autoload,
        },
        compact: format!("{id}.md"),
        compact_content: None,
        full: None,
        full_content: None,
        examples: vec![],
    }
}

#[test]
fn route_ranks_the_explicit_term_skill_first_and_marks_autoload() {
    // The exact concern: two review-pr skills. A "coderabbit" prompt must rank
    // the CodeRabbit skill above the generic one, deterministically.
    let mut skills = BTreeMap::new();
    for s in [
        skill(
            "keel_review_pr_coderabbit",
            &["coderabbit"],
            &["github_pr"],
            true,
        ),
        skill("keel_review_pr_team", &[], &["github_pr"], false),
    ] {
        skills.insert(s.id.clone(), s);
    }
    let snap = Snapshot::build(vec![], BTreeMap::new(), skills, "t".into()).unwrap();

    let routed = route(&snap, "revisá este PR para coderabbit", 8).skills;
    assert_eq!(
        routed[0].id, "keel_review_pr_coderabbit",
        "explicit term wins"
    );
    assert!(routed[0].trigger.contains("coderabbit"));
    assert!(
        routed[0].autoload,
        "autoload skill with a strong match is pushed"
    );
    assert!(
        routed[0].score > routed[1].score,
        "the sibling ranks strictly lower"
    );
    // The generic team skill still surfaces (both are github_pr candidates), but
    // it never wins the coderabbit prompt.
    assert_eq!(routed[1].id, "keel_review_pr_team");
    assert!(!routed[1].autoload);
}

#[test]
fn route_returns_nothing_when_no_capability_matches() {
    let s = skill(
        "keel_review_pr_coderabbit",
        &["coderabbit"],
        &["github_pr"],
        true,
    );
    let mut skills = BTreeMap::new();
    skills.insert(s.id.clone(), s);
    let snap = Snapshot::build(vec![], BTreeMap::new(), skills, "t".into()).unwrap();

    let r = route(&snap, "explain how ownership works in rust", 8);
    assert!(r.is_empty(), "no match → empty, no noise");
}

#[test]
fn route_surfaces_governed_components_like_knowledge() {
    // D-016: not only skills/agents — a governed component (knowledge,
    // workflow, ...) is surfaced at the right moment too.
    use crate::snapshot::CompiledComponent;
    let mut components = BTreeMap::new();
    components.insert(
        "knowledge:flutter_code_rules".to_string(),
        CompiledComponent {
            kind: "knowledge".into(),
            id: "flutter_code_rules".into(),
            version: "0.1.0".into(),
            description: Some("Flutter coding rules: widget classes, Result pattern".into()),
            match_: CompiledMatch {
                terms: vec!["flutter".into()],
                derived: derive_terms(&["flutter_code_rules", "Flutter coding rules"]),
                context: vec![],
                autoload: false,
            },
            content: None,
            inline: None,
            requirements: vec![],
            capabilities: vec![],
            config: None,
        },
    );
    let snap = Snapshot::build_with_components(
        vec![],
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        components,
        "t".into(),
    )
    .unwrap();

    let r = route(&snap, "estoy escribiendo un widget en flutter", 8);
    assert_eq!(r.components.len(), 1, "the flutter knowledge is surfaced");
    assert_eq!(r.components[0].id, "knowledge:flutter_code_rules");
    assert!(r.components[0].trigger.contains("flutter"));
}
