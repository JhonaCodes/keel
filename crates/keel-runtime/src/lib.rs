//! Keel's provider-agnostic cognitive runtime.
//!
//! This crate owns the normalized session protocol. Provider-specific clients
//! are ModelExecutors; they do not own policy, component storage, phase changes,
//! or resource execution.

mod broker;
mod capability;
mod executor;
mod knowledge_chain;
mod mcp_provider;
mod phase;
mod registry;
mod scheduler;
mod store;

pub use executor::{
    CliModelExecutor, ExecutorError, MockModelExecutor, ModelExecutor, ModelMessage, ModelRequest,
    ModelResponse, ToolCall, ToolDefinition, executor_command, executor_env,
};
pub use knowledge_chain::{
    BrokenLink, ChainVerification, KnowledgeChain, KnowledgeChainError, KnowledgeEntry,
};
pub use mcp_provider::{McpProviderError, McpServerSpec, compiled_mcp_providers};
pub use phase::{ArtifactKind, ArtifactReceipt, Phase, PhaseError, PhaseTransitionReceipt};
pub use registry::{ComponentDescriptor, ComponentRegistry};
pub use scheduler::{AgentScheduler, AgentTask, QueueState, SchedulerError};
pub use store::{RuntimeStore, SessionMetadata, StoreError};

use keel_core::ContentHash;
use keel_engine::snapshot::CompiledRequirement;
use keel_engine::snapshot::Snapshot;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use thiserror::Error;

/// Operations that can move a governed session forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    SessionStart,
    ComponentList,
    SkillRead,
    KnowledgeRead,
    PlanSubmit,
    ActionRequest,
    AgentInvoke,
    PhaseAdvance,
    Delivery,
    SessionClose,
}

impl Operation {
    fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "session.start",
            Self::ComponentList => "component.list",
            Self::SkillRead => "skill.read",
            Self::KnowledgeRead => "knowledge.read",
            Self::PlanSubmit => "plan.submit",
            Self::ActionRequest => "action.request",
            Self::AgentInvoke => "agent.invoke",
            Self::PhaseAdvance => "phase.advance",
            Self::Delivery => "delivery",
            Self::SessionClose => "session.close",
        }
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("required components have not been read before {operation}: {missing:?}")]
    RequiredComponentRead {
        operation: String,
        missing: Vec<String>,
    },
    #[error("skill not found: {skill_id}")]
    SkillNotFound { skill_id: String },
    #[error("component not found: {component_kind}:{component_id}")]
    ComponentNotFound {
        component_kind: String,
        component_id: String,
    },
    #[error("full variant is not available for skill: {skill_id}")]
    FullVariantUnavailable { skill_id: String },
    #[error("could not read skill `{skill_id}` at `{path}`: {message}")]
    SkillContentRead {
        skill_id: String,
        path: String,
        message: String,
    },
    #[error("model executor failed: {message}")]
    Executor { message: String },
    #[error(transparent)]
    Store {
        #[from]
        source: StoreError,
    },
    #[error(transparent)]
    Phase {
        #[from]
        source: PhaseError,
    },
    #[error("operation `{operation}` is not allowed in phase {current:?}; allowed: {allowed:?}")]
    WrongPhase {
        operation: String,
        current: Phase,
        allowed: Vec<Phase>,
    },
    #[error("skill.read requested phase `{requested}`, but runtime phase is {current:?}")]
    RequestPhaseMismatch { requested: String, current: Phase },
    #[error("artifact schema is invalid: {message}")]
    ArtifactSchema { message: String },
    #[error("request session `{requested}` does not match runtime session `{expected}`")]
    SessionMismatch { requested: String, expected: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillVariant {
    Compact,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillReadRequest {
    pub skill_id: String,
    pub variant: SkillVariant,
    pub phase: String,
    pub reason: Option<String>,
}

impl SkillReadRequest {
    pub fn compact(skill_id: impl Into<String>, phase: impl Into<String>) -> Self {
        Self {
            skill_id: skill_id.into(),
            variant: SkillVariant::Compact,
            phase: phase.into(),
            reason: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDefinition {
    pub id: String,
    pub version: String,
    pub compact: String,
    pub full: Option<String>,
}

impl SkillDefinition {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        compact: impl Into<String>,
        full: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            compact: compact.into(),
            full,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillReadReceipt {
    pub skill_id: String,
    pub version: String,
    pub content_hash: String,
    pub content: String,
    pub receipt_id: String,
    pub required: bool,
    pub session_id: String,
    pub phase: String,
    pub reason: Option<String>,
    pub read_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentDefinition {
    pub kind: String,
    pub id: String,
    pub version: String,
    pub content: String,
}

impl ComponentDefinition {
    pub fn new(
        kind: impl Into<String>,
        id: impl Into<String>,
        version: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
            version: version.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentReadRequest {
    pub component_kind: String,
    pub component_id: String,
    pub phase: String,
    pub reason: Option<String>,
}

impl ComponentReadRequest {
    pub fn new(kind: impl Into<String>, id: impl Into<String>, phase: impl Into<String>) -> Self {
        Self {
            component_kind: kind.into(),
            component_id: id.into(),
            phase: phase.into(),
            reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComponentReadReceipt {
    pub component_kind: String,
    pub component_id: String,
    pub version: String,
    pub content_hash: String,
    pub content: String,
    pub receipt_id: String,
    pub required: bool,
    pub session_id: String,
    pub phase: String,
    pub reason: Option<String>,
    pub read_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RequiredComponent {
    pub kind: String,
    pub id: String,
    pub reason: Option<String>,
}

/// The first executable slice of the Keel-owned runtime protocol.
///
/// A provider may describe an intended action in text, but only a successful
/// operation handled by this host changes the session state.
pub struct RuntimeHost {
    session_id: String,
    snapshot_hash: String,
    skills: BTreeMap<String, SkillDefinition>,
    components: BTreeMap<(String, String), ComponentDefinition>,
    required_skills: BTreeSet<String>,
    required_components: Vec<RequiredComponent>,
    compiled_requirements: Vec<CompiledRequirement>,
    consumed_skills: BTreeSet<String>,
    consumed_components: BTreeSet<(String, String)>,
    registry: ComponentRegistry,
    store: Option<RuntimeStore>,
    phases: phase::PhaseController,
}

impl RuntimeHost {
    pub fn new(session_id: impl Into<String>, snapshot_hash: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            snapshot_hash: snapshot_hash.into(),
            skills: BTreeMap::new(),
            components: BTreeMap::new(),
            required_skills: BTreeSet::new(),
            required_components: Vec::new(),
            compiled_requirements: Vec::new(),
            consumed_skills: BTreeSet::new(),
            consumed_components: BTreeSet::new(),
            registry: ComponentRegistry::default(),
            store: None,
            phases: phase::PhaseController::default(),
        }
    }

    pub fn with_store(
        session_id: impl Into<String>,
        snapshot_hash: impl Into<String>,
        store: RuntimeStore,
    ) -> Result<Self, RuntimeError> {
        let session_id = session_id.into();
        let snapshot_hash = snapshot_hash.into();
        store.ensure_session(&session_id, &snapshot_hash)?;
        let consumed_skills = store.consumed_skill_ids(&session_id)?.into_iter().collect();
        let consumed_components = store
            .consumed_component_keys(&session_id)?
            .into_iter()
            .collect();
        let phases = phase::PhaseController::restore(
            store.artifacts_for_session(&session_id)?,
            store.transitions_for_session(&session_id)?,
        )?;
        Ok(Self {
            session_id,
            snapshot_hash,
            skills: BTreeMap::new(),
            components: BTreeMap::new(),
            required_skills: BTreeSet::new(),
            required_components: Vec::new(),
            compiled_requirements: Vec::new(),
            consumed_skills,
            consumed_components,
            registry: ComponentRegistry::default(),
            store: Some(store),
            phases,
        })
    }

    pub fn snapshot_hash(&self) -> &str {
        &self.snapshot_hash
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn current_phase(&self) -> Phase {
        self.phases.current()
    }

    pub fn record_artifact(
        &mut self,
        kind: ArtifactKind,
        content: &serde_json::Value,
        schema: &serde_json::Value,
    ) -> Result<ArtifactReceipt, RuntimeError> {
        self.phases.authorize_artifact(kind)?;
        let validator =
            jsonschema::validator_for(schema).map_err(|error| RuntimeError::ArtifactSchema {
                message: error.to_string(),
            })?;
        let receipt = ArtifactReceipt {
            artifact_id: format!("artifact-{}", ulid::Ulid::new()),
            session_id: self.session_id.clone(),
            kind,
            content_hash: ContentHash::of_canonical(content)
                .expect("hashing JSON content cannot fail")
                .to_string(),
            schema_valid: validator.is_valid(content),
            recorded_at: jiff::Timestamp::now().to_string(),
        };
        if let Some(store) = &self.store {
            store.append_artifact(&receipt)?;
        }
        self.phases.record(receipt.clone());
        Ok(receipt)
    }

    pub fn advance_phase(&mut self, target: Phase) -> Result<PhaseTransitionReceipt, RuntimeError> {
        self.authorize_required_components(Operation::PhaseAdvance)?;
        let receipt = self.phases.prepare_transition(&self.session_id, target)?;
        if let Some(store) = &self.store {
            store.append_transition(&receipt)?;
        }
        self.phases.apply(&receipt);
        Ok(receipt)
    }

    pub fn register_skill(&mut self, skill: SkillDefinition) {
        self.registry.register(ComponentDescriptor {
            kind: "skill".to_string(),
            id: skill.id.clone(),
            version: Some(skill.version.clone()),
        });
        self.skills.insert(skill.id.clone(), skill);
    }

    pub fn component_list(&self) -> Vec<ComponentDescriptor> {
        self.registry.list()
    }

    /// Hydrates the runtime registry from Keel's immutable compiled snapshot.
    /// Provider directories are intentionally not consulted.
    pub fn from_snapshot(
        session_id: impl Into<String>,
        snapshot: &Snapshot,
        workspace_root: &Path,
    ) -> Result<Self, RuntimeError> {
        let mut host = Self::new(session_id, snapshot.hash.to_string());
        host.hydrate_snapshot(snapshot, workspace_root)?;
        Ok(host)
    }

    pub fn from_snapshot_with_store(
        session_id: impl Into<String>,
        snapshot: &Snapshot,
        workspace_root: &Path,
        store: RuntimeStore,
    ) -> Result<Self, RuntimeError> {
        let mut host = Self::with_store(session_id, snapshot.hash.to_string(), store)?;
        host.hydrate_snapshot(snapshot, workspace_root)?;
        Ok(host)
    }

    fn hydrate_snapshot(
        &mut self,
        snapshot: &Snapshot,
        workspace_root: &Path,
    ) -> Result<(), RuntimeError> {
        for skill in snapshot.skills.values() {
            let compact_path = workspace_root.join(&skill.compact);
            let compact = std::fs::read_to_string(&compact_path).map_err(|error| {
                RuntimeError::SkillContentRead {
                    skill_id: skill.id.clone(),
                    path: skill.compact.clone(),
                    message: error.to_string(),
                }
            })?;
            let full = skill
                .full
                .as_ref()
                .map(|path| {
                    std::fs::read_to_string(workspace_root.join(path)).map_err(|error| {
                        RuntimeError::SkillContentRead {
                            skill_id: skill.id.clone(),
                            path: path.clone(),
                            message: error.to_string(),
                        }
                    })
                })
                .transpose()?;
            self.register_skill(SkillDefinition::new(
                skill.id.clone(),
                skill.version.clone(),
                compact,
                full,
            ));
        }
        for agent in snapshot.agents.values() {
            self.registry.register(ComponentDescriptor {
                kind: "agent".to_string(),
                id: agent.id.clone(),
                version: None,
            });
        }
        for tool in snapshot.tools.values() {
            self.registry.register(ComponentDescriptor {
                kind: "tool".to_string(),
                id: tool.id.clone(),
                version: None,
            });
        }
        for rule in &snapshot.rules {
            self.registry.register(ComponentDescriptor {
                kind: "rule".to_string(),
                id: rule.id.clone(),
                version: rule.version.clone(),
            });
        }
        for component in snapshot.components.values() {
            self.registry.register(ComponentDescriptor {
                kind: component.kind.clone(),
                id: component.id.clone(),
                version: Some(component.version.clone()),
            });
            self.compiled_requirements
                .extend(component.requirements.iter().cloned());
            let content =
                match (&component.inline, &component.content) {
                    (Some(inline), _) => inline.clone(),
                    (None, Some(path)) => std::fs::read_to_string(workspace_root.join(path))
                        .map_err(|error| RuntimeError::SkillContentRead {
                            skill_id: format!("{}:{}", component.kind, component.id),
                            path: path.clone(),
                            message: error.to_string(),
                        })?,
                    (None, None) => serde_json::to_string(&component.config).unwrap_or_default(),
                };
            self.register_component(ComponentDefinition::new(
                &component.kind,
                &component.id,
                &component.version,
                content,
            ));
        }
        Ok(())
    }

    pub fn require_skill(&mut self, skill_id: impl Into<String>) {
        self.required_skills.insert(skill_id.into());
    }

    pub fn register_component(&mut self, component: ComponentDefinition) {
        self.registry.register(ComponentDescriptor {
            kind: component.kind.clone(),
            id: component.id.clone(),
            version: Some(component.version.clone()),
        });
        self.components
            .insert((component.kind.clone(), component.id.clone()), component);
    }

    pub fn require_component(
        &mut self,
        kind: impl Into<String>,
        id: impl Into<String>,
        reason: Option<String>,
    ) {
        self.required_components.push(RequiredComponent {
            kind: kind.into(),
            id: id.into(),
            reason,
        });
    }

    pub fn read_component(
        &mut self,
        request: ComponentReadRequest,
    ) -> Result<ComponentReadReceipt, RuntimeError> {
        if request.phase != self.current_phase().as_str() {
            return Err(RuntimeError::RequestPhaseMismatch {
                requested: request.phase,
                current: self.current_phase(),
            });
        }
        let key = (request.component_kind.clone(), request.component_id.clone());
        let component =
            self.components
                .get(&key)
                .ok_or_else(|| RuntimeError::ComponentNotFound {
                    component_kind: request.component_kind.clone(),
                    component_id: request.component_id.clone(),
                })?;
        let receipt = ComponentReadReceipt {
            component_kind: component.kind.clone(),
            component_id: component.id.clone(),
            version: component.version.clone(),
            content_hash: ContentHash::of_canonical(&component.content)
                .expect("hashing component content cannot fail")
                .to_string(),
            content: component.content.clone(),
            receipt_id: format!("receipt-{}", ulid::Ulid::new()),
            required: self
                .missing_required_components()
                .iter()
                .any(|required| required.kind == component.kind && required.id == component.id),
            session_id: self.session_id.clone(),
            phase: request.phase,
            reason: request.reason,
            read_at: jiff::Timestamp::now().to_string(),
        };
        if let Some(store) = &self.store {
            store.append_component_receipt(&receipt)?;
        }
        self.consumed_components.insert(key);
        Ok(receipt)
    }

    pub fn read_skill(
        &mut self,
        request: SkillReadRequest,
    ) -> Result<SkillReadReceipt, RuntimeError> {
        if request.phase != self.current_phase().as_str() {
            return Err(RuntimeError::RequestPhaseMismatch {
                requested: request.phase,
                current: self.current_phase(),
            });
        }
        let skill =
            self.skills
                .get(&request.skill_id)
                .ok_or_else(|| RuntimeError::SkillNotFound {
                    skill_id: request.skill_id.clone(),
                })?;
        let content = match request.variant {
            SkillVariant::Compact => skill.compact.clone(),
            SkillVariant::Full => {
                skill
                    .full
                    .clone()
                    .ok_or_else(|| RuntimeError::FullVariantUnavailable {
                        skill_id: request.skill_id.clone(),
                    })?
            }
        };
        let content_hash = ContentHash::of_canonical(&content)
            .expect("hashing a string cannot fail")
            .to_string();
        let receipt = SkillReadReceipt {
            skill_id: skill.id.clone(),
            version: skill.version.clone(),
            content_hash,
            content,
            receipt_id: format!("receipt-{}", ulid::Ulid::new()),
            required: self.missing_required_skill_ids().contains(&skill.id),
            session_id: self.session_id.clone(),
            phase: request.phase,
            reason: request.reason,
            read_at: jiff::Timestamp::now().to_string(),
        };
        if let Some(store) = &self.store {
            store.append_skill_receipt(&receipt)?;
        }
        self.consumed_skills.insert(receipt.skill_id.clone());
        self.consumed_components
            .insert(("skill".to_string(), receipt.skill_id.clone()));
        Ok(receipt)
    }

    pub fn authorize(&self, operation: Operation) -> Result<(), RuntimeError> {
        self.authorize_required_components(operation)?;
        let allowed = match operation {
            Operation::PlanSubmit => Some(vec![Phase::Planning]),
            Operation::ActionRequest => Some(vec![Phase::Implementation]),
            Operation::Delivery => Some(vec![Phase::Delivery]),
            Operation::SessionClose => Some(vec![Phase::Delivery]),
            Operation::SessionStart => Some(vec![Phase::Investigation]),
            _ => None,
        };
        if let Some(allowed) = allowed
            && !allowed.contains(&self.current_phase())
        {
            return Err(RuntimeError::WrongPhase {
                operation: operation.as_str().to_string(),
                current: self.current_phase(),
                allowed,
            });
        }
        Ok(())
    }

    fn authorize_required_components(&self, operation: Operation) -> Result<(), RuntimeError> {
        let missing = self
            .missing_required_components()
            .into_iter()
            .map(|component| format!("{}:{}", component.kind, component.id))
            .collect::<Vec<_>>();
        if !missing.is_empty()
            && matches!(
                operation,
                Operation::PlanSubmit
                    | Operation::ActionRequest
                    | Operation::AgentInvoke
                    | Operation::PhaseAdvance
                    | Operation::Delivery
            )
        {
            return Err(RuntimeError::RequiredComponentRead {
                operation: operation.as_str().to_string(),
                missing,
            });
        }
        Ok(())
    }

    pub fn missing_required_skill_ids(&self) -> Vec<String> {
        self.missing_required_components()
            .into_iter()
            .filter(|component| component.kind == "skill")
            .map(|component| component.id)
            .collect()
    }

    pub fn missing_required_components(&self) -> Vec<RequiredComponent> {
        let phase = self.current_phase().as_str();
        let mut required = BTreeMap::<(String, String), RequiredComponent>::new();
        for id in &self.required_skills {
            required.insert(
                ("skill".to_string(), id.clone()),
                RequiredComponent {
                    kind: "skill".to_string(),
                    id: id.clone(),
                    reason: None,
                },
            );
        }
        for component in &self.required_components {
            required.insert(
                (component.kind.clone(), component.id.clone()),
                component.clone(),
            );
        }
        for requirement in &self.compiled_requirements {
            if requirement.required
                && (requirement.phases.is_empty()
                    || requirement
                        .phases
                        .iter()
                        .any(|candidate| candidate == phase))
            {
                required.insert(
                    (requirement.kind.clone(), requirement.id.clone()),
                    RequiredComponent {
                        kind: requirement.kind.clone(),
                        id: requirement.id.clone(),
                        reason: requirement.reason.clone(),
                    },
                );
            }
        }
        required
            .into_iter()
            .filter_map(|(key, component)| {
                (!self.consumed_components.contains(&key)).then_some(component)
            })
            .collect()
    }

    /// Model text is deliberately not evidence of a protocol operation.
    pub fn note_model_text(&mut self, _text: &str) {}

    /// Sends a normalized request through Keel's executor boundary. The
    /// executor may be Claude, Codex, or a test double; it cannot authorize
    /// operations or access workspace capabilities directly.
    pub fn execute_model_turn<E: ModelExecutor>(
        &mut self,
        executor: &mut E,
        request: ModelRequest,
    ) -> Result<ModelResponse, RuntimeError> {
        if request.session_id != self.session_id {
            return Err(RuntimeError::SessionMismatch {
                requested: request.session_id,
                expected: self.session_id.clone(),
            });
        }
        executor
            .complete(request)
            .map_err(|error| RuntimeError::Executor {
                message: error.to_string(),
            })
    }
}

pub use broker::{AgentBroker, AgentBrokerError, AgentResult};
pub use capability::{CapabilityError, CapabilityManager, CapabilityRequest, CapabilityResult};

#[cfg(test)]
mod tests {
    use super::{
        ComponentDefinition, ComponentDescriptor, ComponentReadRequest, MockModelExecutor,
        ModelExecutor, ModelMessage, ModelRequest, ModelResponse, Operation, RuntimeError,
        RuntimeHost, SkillDefinition, SkillReadRequest,
    };
    use keel_engine::snapshot::{CompiledSkill, Snapshot};
    use std::collections::BTreeMap;

    #[test]
    fn required_skill_blocks_plan_until_read_through_keel() {
        let mut host = RuntimeHost::new("session-1", "sha256:snapshot");
        host.register_skill(SkillDefinition::new(
            "architecture.review",
            "1.0.0",
            "Review architecture before planning.",
            None,
        ));
        host.require_skill("architecture.review");

        let err = host.authorize(Operation::PlanSubmit).unwrap_err();
        assert!(matches!(err, RuntimeError::RequiredComponentRead { .. }));

        let receipt = host
            .read_skill(SkillReadRequest::compact(
                "architecture.review",
                "investigation",
            ))
            .expect("skill read should be served by Keel");
        assert_eq!(receipt.skill_id, "architecture.review");
        assert!(!receipt.content_hash.is_empty());
        assert!(matches!(
            host.authorize(Operation::PlanSubmit),
            Err(RuntimeError::WrongPhase { .. })
        ));
    }

    #[test]
    fn saying_that_a_skill_will_be_read_does_not_satisfy_the_requirement() {
        let mut host = RuntimeHost::new("session-1", "sha256:snapshot");
        host.register_skill(SkillDefinition::new(
            "architecture.review",
            "1.0.0",
            "Review architecture before planning.",
            None,
        ));
        host.require_skill("architecture.review");
        host.note_model_text("I will read architecture.review before planning.");

        assert!(matches!(
            host.authorize(Operation::PlanSubmit),
            Err(RuntimeError::RequiredComponentRead { .. })
        ));
    }

    #[test]
    fn snapshot_hydrates_skills_inside_keel_runtime() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("compact.md"), "Keel-owned guidance").unwrap();
        let mut skills = BTreeMap::new();
        skills.insert(
            "architecture.review".to_string(),
            CompiledSkill {
                id: "architecture.review".to_string(),
                description: None,
                match_: Default::default(),
                version: "1.2.0".to_string(),
                compact: "compact.md".to_string(),
                full: None,
                examples: Vec::new(),
            },
        );
        let snapshot = Snapshot::build(Vec::new(), BTreeMap::new(), skills, "now".into()).unwrap();
        let mut host = RuntimeHost::from_snapshot("session-2", &snapshot, root.path()).unwrap();
        host.require_skill("architecture.review");
        let mut request = SkillReadRequest::compact("architecture.review", "investigation");
        request.reason = Some("reference requirement".to_string());
        let receipt = host.read_skill(request).unwrap();

        assert_eq!(receipt.version, "1.2.0");
        assert_eq!(receipt.content, "Keel-owned guidance");
        assert_eq!(receipt.session_id, "session-2");
        assert_eq!(receipt.reason.as_deref(), Some("reference requirement"));
        assert!(!receipt.read_at.is_empty());
    }

    #[test]
    fn model_executor_is_called_through_normalized_runtime_boundary() {
        let mut host = RuntimeHost::new("session-3", "sha256:snapshot");
        let mut executor = MockModelExecutor::with_response(ModelResponse::text("structured"));
        let response = host
            .execute_model_turn(
                &mut executor,
                ModelRequest::new("session-3", vec![ModelMessage::user("prompt")]),
            )
            .unwrap();

        assert_eq!(response.content, "structured");
        assert_eq!(executor.provider_id(), "mock");
        assert_eq!(executor.requests().len(), 1);
    }

    #[test]
    fn model_request_for_another_session_never_reaches_the_executor() {
        let mut host = RuntimeHost::new("session-3", "sha256:snapshot");
        let mut executor = MockModelExecutor::with_response(ModelResponse::text("unexpected"));

        assert!(matches!(
            host.execute_model_turn(
                &mut executor,
                ModelRequest::new("another-session", vec![ModelMessage::user("prompt")]),
            ),
            Err(RuntimeError::SessionMismatch { .. })
        ));
        assert!(executor.requests().is_empty());
    }

    #[test]
    fn registry_is_open_ended_and_lists_snapshot_components() {
        let mut host = RuntimeHost::new("session-4", "sha256:snapshot");
        host.register_skill(SkillDefinition::new("skill-a", "1.0.0", "a", None));
        host.registry.register(ComponentDescriptor {
            kind: "knowledge".to_string(),
            id: "knowledge-a".to_string(),
            version: Some("1.0.0".to_string()),
        });

        assert!(host.registry.contains("knowledge", "knowledge-a"));
        assert_eq!(host.component_list().len(), 2);
    }

    #[test]
    fn governed_component_read_records_real_consumption() {
        let mut host = RuntimeHost::new("session-components", "sha256:snapshot");
        host.register_component(ComponentDefinition::new(
            "knowledge",
            "architecture",
            "1.0.0",
            "Keel-owned architecture context",
        ));
        host.require_component("knowledge", "architecture", None);

        assert!(matches!(
            host.authorize(Operation::PlanSubmit),
            Err(RuntimeError::RequiredComponentRead { .. })
        ));
        let receipt = host
            .read_component(ComponentReadRequest::new(
                "knowledge",
                "architecture",
                "investigation",
            ))
            .unwrap();
        assert_eq!(receipt.component_kind, "knowledge");
        assert_eq!(receipt.component_id, "architecture");
        assert!(host.missing_required_components().is_empty());
    }
}
