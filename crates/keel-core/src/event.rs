// SPDX-License-Identifier: Apache-2.0
//! The protocol's reserved events (spec section 11.2) and the event envelope the
//! runtime consumes.
//!
//! WHY IT LIVES IN CORE AND NOT IN THE DSL: the event is PROTOCOL vocabulary
//! (adapter ⇄ runtime), not authoring vocabulary. The runtime evaluates events
//! without ever knowing the DSL (forbidden edge `runtime ⇏ dsl`); that is why
//! the shared type lives in the leaf both can see.
//!
//! PHASE NOTE (spec section 6.2, invariant 17): phase events
//! (`analysis.started`, `implementation.started`, …) are emitted by the
//! RUNTIME when it authorizes the transition — the model does not declare its
//! own phase. In Phase 0 (passive replay) they may appear in fixtures as
//! already-authorized events.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The 17 reserved events (spec section 11.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EventKind {
    #[serde(rename = "session.started")]
    SessionStarted,
    #[serde(rename = "prompt.submitted")]
    PromptSubmitted,
    #[serde(rename = "analysis.started")]
    AnalysisStarted,
    #[serde(rename = "context.requested")]
    ContextRequested,
    #[serde(rename = "file.opened")]
    FileOpened,
    #[serde(rename = "file.edited")]
    FileEdited,
    #[serde(rename = "command.requested")]
    CommandRequested,
    #[serde(rename = "command.completed")]
    CommandCompleted,
    #[serde(rename = "dependency.changed")]
    DependencyChanged,
    #[serde(rename = "transition.requested")]
    TransitionRequested,
    #[serde(rename = "implementation.started")]
    ImplementationStarted,
    #[serde(rename = "verification.started")]
    VerificationStarted,
    #[serde(rename = "test.completed")]
    TestCompleted,
    #[serde(rename = "audit.started")]
    AuditStarted,
    #[serde(rename = "completion.requested")]
    CompletionRequested,
    #[serde(rename = "delivery.requested")]
    DeliveryRequested,
    #[serde(rename = "session.ended")]
    SessionEnded,
}

impl EventKind {
    /// Inner ring (spec section 5.3): potentially irreversible actions whose
    /// interception is ALWAYS pre-action. In Phase 0 (passive) this is living
    /// documentation + telemetry data; in Phase 1 it governs blocking.
    pub fn is_inner_ring(self) -> bool {
        matches!(
            self,
            EventKind::CommandRequested
                | EventKind::TransitionRequested
                | EventKind::DeliveryRequested
        )
    }
}

/// Event envelope as delivered by the adapter (Phase 0: JSONL replay).
///
/// REPLAY HONESTY (ADR-022): `preconditions` evaluate the state of the world
/// AT THE MOMENT OF THE REQUEST. In replay that state is captured inside the
/// event itself (`env`); a live adapter (Phase 1) may probe the real
/// environment. That is why `env` is part of the envelope and not a query by
/// the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub kind: EventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Affected file (file.* / dependency.changed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Content or diff of the edited file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Main line of the change (oscillation key per section 6.5 together with `file`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Requested command (command.requested — inner ring).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Environment state captured at the moment of the request (ADR-022).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Files touched by the task (for `when.files.touch` in phase events).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

#[cfg(test)]
#[path = "../tests-unit/event.rs"]
mod tests;