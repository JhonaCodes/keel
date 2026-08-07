use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Investigation,
    Planning,
    Implementation,
    Verification,
    Audit,
    Resolution,
    Acceptance,
    Delivery,
}

impl Phase {
    pub fn all_non_terminal() -> impl Iterator<Item = Self> {
        [
            Self::Investigation,
            Self::Planning,
            Self::Implementation,
            Self::Verification,
            Self::Audit,
            Self::Resolution,
            Self::Acceptance,
        ]
        .into_iter()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Investigation => "investigation",
            Self::Planning => "planning",
            Self::Implementation => "implementation",
            Self::Verification => "verification",
            Self::Audit => "audit",
            Self::Resolution => "resolution",
            Self::Acceptance => "acceptance",
            Self::Delivery => "delivery",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "investigation" => Some(Self::Investigation),
            "planning" => Some(Self::Planning),
            "implementation" => Some(Self::Implementation),
            "verification" => Some(Self::Verification),
            "audit" => Some(Self::Audit),
            "resolution" => Some(Self::Resolution),
            "acceptance" => Some(Self::Acceptance),
            "delivery" => Some(Self::Delivery),
            _ => None,
        }
    }

    fn next(self) -> Option<Self> {
        match self {
            Self::Investigation => Some(Self::Planning),
            Self::Planning => Some(Self::Implementation),
            Self::Implementation => Some(Self::Verification),
            Self::Verification => Some(Self::Audit),
            Self::Audit => Some(Self::Resolution),
            Self::Resolution => Some(Self::Acceptance),
            Self::Acceptance => Some(Self::Delivery),
            Self::Delivery => None,
        }
    }

    fn required_artifact(self) -> Option<ArtifactKind> {
        match self {
            Self::Investigation => Some(ArtifactKind::InvestigationReport),
            Self::Planning => Some(ArtifactKind::SolutionContract),
            Self::Implementation => Some(ArtifactKind::ImplementationRecord),
            Self::Verification => Some(ArtifactKind::EvidenceReport),
            Self::Audit => Some(ArtifactKind::AuditReport),
            Self::Resolution => Some(ArtifactKind::ResolutionRecord),
            Self::Acceptance => Some(ArtifactKind::AcceptanceRecord),
            Self::Delivery => None,
        }
    }

    fn accepts_artifact(self, kind: ArtifactKind) -> bool {
        match self {
            Self::Investigation => kind == ArtifactKind::InvestigationReport,
            Self::Planning => kind == ArtifactKind::SolutionContract,
            Self::Implementation => kind == ArtifactKind::ImplementationRecord,
            Self::Verification => kind == ArtifactKind::EvidenceReport,
            Self::Audit => kind == ArtifactKind::AuditReport,
            Self::Resolution => matches!(
                kind,
                ArtifactKind::CorrectionContract | ArtifactKind::ResolutionRecord
            ),
            Self::Acceptance => kind == ArtifactKind::AcceptanceRecord,
            Self::Delivery => kind == ArtifactKind::DeliveryRecord,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    InvestigationReport,
    SolutionContract,
    ImplementationRecord,
    EvidenceReport,
    AuditReport,
    CorrectionContract,
    ResolutionRecord,
    AcceptanceRecord,
    DeliveryRecord,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvestigationReport => "investigation-report",
            Self::SolutionContract => "solution-contract",
            Self::ImplementationRecord => "implementation-record",
            Self::EvidenceReport => "evidence-report",
            Self::AuditReport => "audit-report",
            Self::CorrectionContract => "correction-contract",
            Self::ResolutionRecord => "resolution-record",
            Self::AcceptanceRecord => "acceptance-record",
            Self::DeliveryRecord => "delivery-record",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "investigation-report" => Some(Self::InvestigationReport),
            "solution-contract" => Some(Self::SolutionContract),
            "implementation-record" => Some(Self::ImplementationRecord),
            "evidence-report" => Some(Self::EvidenceReport),
            "audit-report" => Some(Self::AuditReport),
            "correction-contract" => Some(Self::CorrectionContract),
            "resolution-record" => Some(Self::ResolutionRecord),
            "acceptance-record" => Some(Self::AcceptanceRecord),
            "delivery-record" => Some(Self::DeliveryRecord),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactReceipt {
    pub artifact_id: String,
    pub session_id: String,
    pub kind: ArtifactKind,
    pub content_hash: String,
    pub schema_valid: bool,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhaseTransitionReceipt {
    pub transition_id: String,
    pub session_id: String,
    pub from: Phase,
    pub to: Phase,
    pub guard_artifact_id: String,
    pub advanced_at: String,
}

#[derive(Debug, Error)]
pub enum PhaseError {
    #[error("cannot transition from {from:?} to {to:?}")]
    InvalidTransition { from: Phase, to: Phase },
    #[error("phase {phase:?} requires a valid {artifact:?}")]
    RequiredArtifactMissing {
        phase: Phase,
        artifact: ArtifactKind,
    },
    #[error("artifact {artifact:?} is not allowed in phase {phase:?}")]
    ArtifactNotAllowed {
        phase: Phase,
        artifact: ArtifactKind,
    },
    #[error("stored phase history is inconsistent at {from:?} -> {to:?}")]
    InvalidHistory { from: Phase, to: Phase },
}

pub(crate) struct PhaseController {
    current: Phase,
    artifacts: BTreeMap<ArtifactKind, Vec<ArtifactReceipt>>,
}

impl Default for PhaseController {
    fn default() -> Self {
        Self {
            current: Phase::Investigation,
            artifacts: BTreeMap::new(),
        }
    }
}

impl PhaseController {
    pub(crate) fn restore(
        artifacts: Vec<ArtifactReceipt>,
        transitions: Vec<PhaseTransitionReceipt>,
    ) -> Result<Self, PhaseError> {
        let mut controller = Self::default();
        for artifact in artifacts {
            controller.record(artifact);
        }
        for transition in transitions {
            let required = controller.current.required_artifact();
            let valid_guard = required
                .and_then(|kind| controller.artifacts.get(&kind))
                .is_some_and(|receipts| {
                    receipts.iter().any(|receipt| {
                        receipt.schema_valid && receipt.artifact_id == transition.guard_artifact_id
                    })
                });
            if transition.from != controller.current
                || controller.current.next() != Some(transition.to)
                || !valid_guard
            {
                return Err(PhaseError::InvalidHistory {
                    from: transition.from,
                    to: transition.to,
                });
            }
            controller.current = transition.to;
        }
        Ok(controller)
    }

    pub(crate) fn current(&self) -> Phase {
        self.current
    }

    pub(crate) fn record(&mut self, receipt: ArtifactReceipt) {
        self.artifacts
            .entry(receipt.kind)
            .or_default()
            .push(receipt);
    }

    pub(crate) fn authorize_artifact(&self, kind: ArtifactKind) -> Result<(), PhaseError> {
        if !self.current.accepts_artifact(kind) {
            return Err(PhaseError::ArtifactNotAllowed {
                phase: self.current,
                artifact: kind,
            });
        }
        Ok(())
    }

    pub(crate) fn prepare_transition(
        &self,
        session_id: &str,
        target: Phase,
    ) -> Result<PhaseTransitionReceipt, PhaseError> {
        if self.current.next() != Some(target) {
            return Err(PhaseError::InvalidTransition {
                from: self.current,
                to: target,
            });
        }
        let required = self
            .current
            .required_artifact()
            .expect("the terminal phase has no outgoing transition");
        let artifact = self
            .artifacts
            .get(&required)
            .and_then(|receipts| receipts.iter().rev().find(|receipt| receipt.schema_valid))
            .ok_or(PhaseError::RequiredArtifactMissing {
                phase: self.current,
                artifact: required,
            })?;
        Ok(PhaseTransitionReceipt {
            transition_id: format!("transition-{}", ulid::Ulid::new()),
            session_id: session_id.to_string(),
            from: self.current,
            to: target,
            guard_artifact_id: artifact.artifact_id.clone(),
            advanced_at: jiff::Timestamp::now().to_string(),
        })
    }

    pub(crate) fn apply(&mut self, transition: &PhaseTransitionReceipt) {
        self.current = transition.to;
    }
}
