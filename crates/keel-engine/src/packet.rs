// SPDX-License-Identifier: Apache-2.0
//! ContextPacket — the ONLY thing the model ever receives (spec section 10.4).
//!
//! ~50–100 tokens, adjacent to the action it governs, at the exact turn where
//! it applies: the verdict, the constraint, the required action, an exemplar
//! pair when available, and the evidence id. Never the YAML, never workspace
//! paths, never the composition tree (ADR-004).
//!
//! A blocking packet must reduce ambiguity to near zero (section 6.5): a block
//! whose message is open to interpretation reproduces the failure mode the
//! system exists to prevent.
//!
//! BOUNDARY RULE: does not import `keel_dsl` — built from compiled artifacts.

use crate::runtime::Evaluation;
use keel_core::Decision;

/// Renders the transcript-facing packet for one evaluation (section 10.4 format).
///
/// `skill_payload` is the L2 cognitive-activation content (compact/full skill
/// text + exemplar), delivered once per session by the Session Manager; empty
/// when the skill is already loaded or the rule loads nothing.
pub fn render(
    eval: &Evaluation,
    ev_id: &str,
    skill_payload: &[String],
    snapshot_hash: &str,
) -> String {
    // Rendered as a plain-ASCII framed block so the OPERATOR reads it at a
    // glance in the transcript, while staying compact and parseable for the
    // model: the canonical tokens `BLOCKED (<rule>)`, `source: rule=…`,
    // `Evidence: …` remain verbatim on their own lines. No emojis or symbols
    // the operator might not recognize.
    let title = match eval.effective_decision {
        Decision::Block => format!("BLOCKED ({})", eval.rule_id),
        Decision::DenyPendingApproval => format!("DENIED - pending approval ({})", eval.rule_id),
        Decision::Review => format!("REVIEW ({})", eval.rule_id),
        Decision::Allow => format!("OK ({})", eval.rule_id),
    };

    let mut body: Vec<String> = vec![title];

    // Why: findings first (localized), then the rule's own detail.
    for f in &eval.findings {
        match (&f.file, f.line) {
            (Some(file), Some(line)) => body.push(format!("- {} ({file}:{line})", f.message)),
            _ => body.push(format!("- {}", f.message)),
        }
    }
    if let Some(detail) = &eval.detail {
        body.push(format!("- {detail}"));
    }

    // section 4.7 / ADR-017: uncertainty over an irreversible action escalates
    // to a human — say so explicitly so the model does not retry.
    if eval.effective_decision == Decision::DenyPendingApproval {
        body.push(
            "- requires human approval - a model never authorizes an irreversible action (section 4.7)"
                .to_string(),
        );
    }

    // What to do next: the L2 skill content / exemplar, if delivered.
    for chunk in skill_payload {
        body.push(format!("-> {chunk}"));
    }

    // Source + evidence (section 10.4): reproducible and citable, without
    // exposing the rule body or workspace paths (ADR-004).
    body.push(format!(
        "source: rule={} snapshot={snapshot_hash}",
        eval.rule_id
    ));
    body.push(format!("Evidence: {ev_id} logged"));

    frame(&body)
}

/// Wraps lines in an ASCII `keel` frame, so a block reads as one distinct unit
/// in the transcript. `+-- keel --...--+` on top, indented body, matching
/// bottom rule.
fn frame(lines: &[String]) -> String {
    const LABEL: &str = "+-- keel ";
    let width = lines
        .iter()
        .flat_map(|l| l.lines())
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(LABEL.len(), 100);
    let top = format!(
        "{LABEL}{}+",
        "-".repeat(width.saturating_sub(LABEL.len()) + 3)
    );
    let bottom = format!("+{}+", "-".repeat(width + 2));
    let mut out = String::new();
    out.push_str(&top);
    out.push('\n');
    for line in lines {
        for wrapped in line.lines() {
            out.push_str("  ");
            out.push_str(wrapped);
            out.push('\n');
        }
    }
    out.push_str(&bottom);
    out
}

#[cfg(test)]
#[path = "../tests-unit/packet.rs"]
mod tests;
