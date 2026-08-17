// SPDX-License-Identifier: Apache-2.0
//! Declarative capability routing (D-014) — the shared vocabulary that decides
//! WHICH governed skills/agents relate to a prompt, deterministically and
//! explainably.
//!
//! One home for both halves so the two never drift:
//! - COMPILE time ([`derive_terms`], [`context_for_term`]): the compiler folds
//!   terms derived from a capability's id + description into its `CompiledMatch`.
//! - RUN time ([`detect_contexts`], [`score`], [`route`]): the gate scores a
//!   prompt against every compiled capability and returns a ranked shortlist —
//!   the native replacement for a workspace bag-of-words catalog.
//!
//! Determinism is the point: a routing decision is a set-intersection with a
//! fixed weighting, so keel can always say WHY a capability was surfaced (the
//! trigger that fired), which a semantic/embedding matcher could not.

use crate::snapshot::{CompiledMatch, Snapshot};
use std::collections::BTreeSet;

/// Stop-words dropped from derived terms: English + Spanish FUNCTION words plus
/// Keel STRUCTURAL/provenance tokens (`keel`, `skill`, `agent`, `workflow`) that
/// appear in ids as scaffolding, never as routing intent. Domain words like
/// `review`, `pr`, `commit`, `test` are deliberately NOT here — they are the
/// signal. Kept tiny: derivation is a low-weight fallback; explicit `match.terms`
/// is the precision lever, so over-tuning this list buys little.
pub const DERIVE_STOPWORDS: &[&str] = &[
    // English function words.
    "the", "and", "for", "with", "of", "to", "in", "is", "it", "or", "by", "as", "at", "on", "an",
    // Spanish function words.
    "que", "con", "los", "las", "una", "por", "del", "este", "esta", "como", "para", "de", "el",
    "la", "en", "un", "me", "se", "lo", "su", "al",
    // Keel structural / provenance tokens.
    "keel", "skill", "agent", "workflow",
];

/// Keyword → structured object type. Deterministic and documented: a `context`
/// is only inferred from these explicit cues, never guessed, so the
/// highest-weight signal stays predictable and auditable.
///
/// Two families live here. TICKET-shaped cues answer "what object is this
/// about". TECHNOLOGY cues answer "what platform is this code", which is what
/// lets a `platforms/<tech>/` layer's agents and skills win the top weight on a
/// code moment: without them a `context: [platform/flutter]` block could never
/// fire, since the moment text for a file edit is the file's own path/content
/// and carried no platform signal at all.
///
/// The technology cues are deliberately narrow. `dart` maps to the LANGUAGE
/// layer, not to Flutter — a Dart backend file is not a Flutter file — while
/// `flutter`/`widget`/`pubspec` mark the framework. Ambiguous short tokens are
/// left out on purpose: `rs` would fire on any Dart file importing `logger_rs`.
pub const CONTEXT_CUES: &[(&str, &str)] = &[
    // Ticket-shaped objects.
    ("pr", "github_pr"),
    ("pull", "github_pr"),
    ("issue", "github_issue"),
    ("linear", "linear_ticket"),
    ("ticket", "linear_ticket"),
    ("jira", "jira_issue"),
    // Technology platforms.
    ("dart", "platform/dart"),
    ("flutter", "platform/flutter"),
    ("widget", "platform/flutter"),
    ("pubspec", "platform/flutter"),
    ("rust", "platform/rust"),
    ("cargo", "platform/rust"),
];

/// The structured object type a cue word implies, if any.
pub fn context_for_term(term: &str) -> Option<&'static str> {
    CONTEXT_CUES
        .iter()
        .find(|(cue, _)| *cue == term)
        .map(|(_, ctx)| *ctx)
}

/// Tokenizes free text into lowercase terms (len > 1, no stop-words), splitting
/// on any non-alphanumeric char so an id like `keel_review_pr` contributes
/// `review`, `pr`. Order-preserving and deduped.
pub fn derive_terms(texts: &[&str]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for text in texts {
        for raw in text.split(|c: char| !c.is_ascii_alphanumeric()) {
            let w = raw.to_ascii_lowercase();
            // len > 1 keeps short domain keywords (`pr`, `ci`, `ui`, `db`); the
            // stop-word list removes 2-char function words (`de`, `el`, `to`…).
            if w.len() > 1 && !DERIVE_STOPWORDS.contains(&w.as_str()) && seen.insert(w.clone()) {
                out.push(w);
            }
        }
    }
    out
}

/// The structured object types a prompt references. Two deterministic sources:
/// explicit URLs (a GitHub PR/issue link, a Linear link) and the cue words of
/// [`CONTEXT_CUES`]. Highest-precision routing signal.
///
/// Called with the moment text, which on a file edit is the file's content or
/// its path — so the technology cues resolve the platform straight off an
/// `import 'package:flutter/...'` or a `lib/foo.dart` path.
pub fn detect_contexts(prompt: &str) -> BTreeSet<String> {
    let low = prompt.to_ascii_lowercase();
    let mut ctx = BTreeSet::new();
    // Structured URLs win regardless of surrounding words.
    if low.contains("/pull/") || low.contains("pull request") {
        ctx.insert("github_pr".to_string());
    }
    if low.contains("/issues/") {
        ctx.insert("github_issue".to_string());
    }
    if low.contains("linear.app") {
        ctx.insert("linear_ticket".to_string());
    }
    // Cue words (`pr`, `linear`, `ticket`, …).
    for term in derive_terms(&[prompt]) {
        if let Some(c) = context_for_term(&term) {
            ctx.insert(c.to_string());
        }
    }
    ctx
}

/// A capability surfaced by routing, with the trigger that won it (for the
/// operator-facing banner) and how it should be delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routed {
    pub id: String,
    pub score: u32,
    /// Human-readable reason this surfaced (`context:github_pr`, `term:coderabbit`).
    pub trigger: String,
    /// Declared-condition count — the specificity tie-breaker.
    pub specificity: usize,
    pub autoload: bool,
}

/// Scores a compiled `match` against a prompt's words and detected contexts.
/// Weighting encodes the doctrine: a structured-object match (3) beats an
/// explicit term (2) beats a derived term (1). Returns the score and the single
/// strongest trigger for the banner, or `None` when nothing matches.
pub fn score(
    prompt_words: &BTreeSet<String>,
    contexts: &BTreeSet<String>,
    m: &CompiledMatch,
) -> Option<(u32, String)> {
    // Score sums weights (context 3 > term 2 > derived 1) for RANKING. The
    // trigger LABEL, though, prefers the most INTENTIONAL signal — an authored
    // explicit term over an inferred context over a derived word — because that
    // is what explains why this capability beat a sibling ("term:coderabbit"
    // says more to the operator than a "context:github_pr" both share).
    let mut total = 0u32;
    let mut term_label: Option<String> = None;
    let mut context_label: Option<String> = None;
    let mut derived_label: Option<String> = None;
    for c in &m.context {
        if contexts.contains(c) {
            total += 3;
            context_label.get_or_insert_with(|| format!("context:{c}"));
        }
    }
    for t in &m.terms {
        if prompt_words.contains(&t.to_ascii_lowercase()) {
            total += 2;
            term_label.get_or_insert_with(|| format!("term:{t}"));
        }
    }
    for d in &m.derived {
        if prompt_words.contains(d) {
            total += 1;
            derived_label.get_or_insert_with(|| format!("term:{d}"));
        }
    }
    let trigger = term_label.or(context_label).or(derived_label)?;
    Some((total, trigger))
}

/// The ranked, relevant capabilities for one moment: skills, agents and other
/// governed components (knowledge, workflows…). Everything the gate
/// turns into `additionalContext` — "you're about to touch X, keel has these".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteResult {
    pub skills: Vec<Routed>,
    pub agents: Vec<Routed>,
    pub components: Vec<Routed>,
}

impl RouteResult {
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty() && self.agents.is_empty() && self.components.is_empty()
    }
}

/// Ranks every skill, agent and governed component in the snapshot against
/// `moment` (the prompt text, the edited file's content, or a command — whatever
/// text describes the current moment), returning at most `limit` of each, most
/// relevant first. Ties break by specificity (more declared conditions wins),
/// then id (stable). A capability with no match is dropped. Deterministic core.
pub fn route(snapshot: &Snapshot, moment: &str, limit: usize) -> RouteResult {
    let words: BTreeSet<String> = derive_terms(&[moment]).into_iter().collect();
    let contexts = detect_contexts(moment);

    let rank = |matches: Vec<Routed>| -> Vec<Routed> {
        let mut v = matches;
        v.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(b.specificity.cmp(&a.specificity))
                .then(a.id.cmp(&b.id))
        });
        v.truncate(limit);
        v
    };

    let skills = rank(
        snapshot
            .skills
            .values()
            .filter_map(|s| {
                score(&words, &contexts, &s.match_).map(|(sc, trigger)| Routed {
                    id: s.id.clone(),
                    score: sc,
                    trigger,
                    specificity: s.match_.terms.len() + s.match_.context.len(),
                    // Autoload only earns a push on a STRONG match (a structured
                    // context or an explicit term, score ≥ 2) — a derived-term
                    // brush is enough to expose, never to inject.
                    autoload: s.match_.autoload && sc >= 2,
                })
            })
            .collect(),
    );
    let agents = rank(
        snapshot
            .agents
            .values()
            .filter_map(|a| {
                score(&words, &contexts, &a.match_).map(|(sc, trigger)| Routed {
                    id: a.id.clone(),
                    score: sc,
                    trigger,
                    specificity: a.match_.terms.len() + a.match_.context.len(),
                    autoload: false, // agents are invoked explicitly, never pushed
                })
            })
            .collect(),
    );
    let components = rank(
        snapshot
            .components
            .values()
            // ModelExecutors are wiring, not something to surface to the model.
            .filter(|c| c.kind != "model-executor")
            .filter_map(|c| {
                score(&words, &contexts, &c.match_).map(|(sc, trigger)| Routed {
                    id: format!("{}:{}", c.kind, c.id),
                    score: sc,
                    trigger,
                    specificity: c.match_.terms.len() + c.match_.context.len(),
                    autoload: false,
                })
            })
            .collect(),
    );
    RouteResult {
        skills,
        agents,
        components,
    }
}

#[cfg(test)]
#[path = "../tests-unit/routing.rs"]
mod tests;
