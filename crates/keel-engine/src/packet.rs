// SPDX-License-Identifier: Apache-2.0
//! ContextPacket — the ONLY thing the model ever receives (spec section 10.4).
//!
//! ~50–100 tokens, adjacent to the action it governs, at the exact turn where
//! it applies: the verdict, the constraint, the required action, an exemplar
//! pair when available, and the evidence id. Never the YAML, never workspace
//! paths, never the composition tree (ADR-004).
//!
//! A blocking packet must reduce ambiguity to near zero (section 6.5): a block whose
//! message is open to interpretation reproduces the failure mode the system
//! exists to prevent.
//!
//! BOUNDARY RULE: does not import `keel_dsl` — built from compiled artifacts.

use crate::runtime::Evaluation;
use keel_core::Decision;

/// Renders the transcript-facing packet for one evaluation (section 10.4 format).
///
/// `skill_payload` is the L2 cognitive-activation content (compact/full skill
/// text + exemplar), delivered once per session by the Session Manager; empty
/// when the skill is already loaded or the rule loads nothing.
pub fn render(eval: &Evaluation, ev_id: &str, skill_payload: &[String]) -> String {
    let mut lines = Vec::new();

    let header = match eval.effective_decision {
        Decision::Block => format!("BLOCKED ({})", eval.rule_id),
        Decision::DenyPendingApproval => format!("DENIED — pending approval ({})", eval.rule_id),
        Decision::Review => format!("REVIEW ({})", eval.rule_id),
        Decision::Allow => format!("OK ({})", eval.rule_id),
    };
    lines.push(header);

    // The constraint and what happened: findings first (localized), then the
    // rule's own detail (report message / failed precondition).
    for f in &eval.findings {
        match (&f.file, f.line) {
            (Some(file), Some(line)) => lines.push(format!("{} — {file}:{line}", f.message)),
            _ => lines.push(f.message.clone()),
        }
    }
    if let Some(detail) = &eval.detail {
        lines.push(detail.clone());
    }

    // section 4.7 / ADR-017: uncertainty over an irreversible action escalates to a
    // human — the packet says so explicitly, so the model does not retry.
    if eval.effective_decision == Decision::DenyPendingApproval {
        lines.push(
            "requires human approval — a model never authorizes an irreversible action (section 4.7)"
                .to_string(),
        );
    }

    // L2 payload (skill content / exemplar), delivered by the Session Manager.
    for chunk in skill_payload {
        lines.push(chunk.clone());
    }

    lines.push(format!("Evidence: {ev_id} logged"));
    lines.join("\n")
}

#[cfg(test)]
#[path = "../tests-unit/packet.rs"]
mod tests;