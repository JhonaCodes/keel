// SPDX-License-Identifier: Apache-2.0
//! keel-core — Keel's stable vocabulary.
//!
//! Contains the types that MUST NOT diverge between compiler and runtime:
//! - [`Verdict`]: three-state verdicts (spec §4.6).
//! - [`OriginClass`]: evidence origin classes (spec §6.4).
//! - [`Decision`]: the decision lattice with its ordering (spec §7.4-D3).
//! - [`Reversibility`]: the dimension that decides the fate of `unknown` (§4.7).
//! - [`ContentHash`]: the ONLY canonical hashing authority (invariant 9).
//! - [`event`]: the protocol's reserved events (spec §11.2).
//!
//! This crate is a deliberate leaf: it knows neither the authoring DSL nor the engine.

pub mod event;
mod hash;

pub use hash::ContentHash;

use serde::{Deserialize, Serialize};

/// Three-state verdict (spec §4.6).
///
/// A tool is never forced to decide: its contract is honest. `Unknown`
/// means "undecidable with the available analysis" and is NEVER an engine
/// error — it is a telemetry signal (poorly specified rule, §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Valid,
    Invalid,
    Unknown,
}

/// Origin class of an evidence entry (spec §6.4).
///
/// The ledger records HOW something was known, not just what was known.
/// "phpstan proved it" (deterministic) is never mixed with "a model has an
/// opinion" (semantic). A system that mixes both classes in its record is
/// lying to itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OriginClass {
    /// Produced by a model-free tool (input hash, version, verdict).
    Deterministic,
    /// Produced by an llm-evaluator (Phase 2 — does not run in Phase 0).
    Semantic,
    /// Asserted by an evaluator or a human about a non-observable condition.
    Attestation,
    /// Explicit human decision (approval, rejection, exception, prune).
    Human,
}

/// The decision lattice (spec §7.4-D3, ADR-017).
///
/// ```text
/// allow < review < block                               (reversible actions)
/// allow < review < block < deny-pending-approval      (irreversible actions)
/// ```
///
/// The derived ordering uses the DECLARATION ORDER of the variants — do NOT
/// reorder them. This is the single definition of the ordering: the compiler
/// uses it (D3 monotonicity check, Phase 1+) and so does the runtime (Phase 0
/// passive forcing). Two definitions could diverge; that is why it lives here.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    Allow,
    Review,
    Block,
    /// Floor of the `unknown` branch for irreversible actions (§4.7): escalates
    /// to a human, never to a model. An LLM never authorizes an action it
    /// could be irreversibly wrong about.
    DenyPendingApproval,
}

/// Reversibility of the governed action (spec §4.7, ADR-017).
///
/// Decides where `unknown` lands on the ladder: reversible → `review`;
/// irreversible → `deny-pending-approval` and a human.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reversibility {
    Reversible,
    Irreversible,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §7.4-D3 lattice is load-bearing: passive forcing and the future
    /// D3 check depend on this exact ordering.
    #[test]
    fn decision_lattice_order() {
        assert!(Decision::Allow < Decision::Review);
        assert!(Decision::Review < Decision::Block);
        assert!(Decision::Block < Decision::DenyPendingApproval);
    }

    #[test]
    fn decision_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_string(&Decision::DenyPendingApproval).unwrap(),
            "\"deny-pending-approval\""
        );
    }

    #[test]
    fn verdict_and_origin_serialize_lowercase() {
        assert_eq!(serde_json::to_string(&Verdict::Unknown).unwrap(), "\"unknown\"");
        assert_eq!(
            serde_json::to_string(&OriginClass::Deterministic).unwrap(),
            "\"deterministic\""
        );
    }
}