// SPDX-License-Identifier: Apache-2.0
//! keel-dsl — Keel's authoring vocabulary (spec section 11).
//!
//! Models the `apiVersion/kind/metadata/spec` envelope and the core kinds
//! Phase 0 needs: `Workspace`, `Rule`, `Tool`, `RuleTest`.
//!
//! The loading flow is ALWAYS: raw YAML → JSON-Schema validation
//! ([`schema`]) → typed deserialization. The schema is the Phase 0a
//! deliverable (spec section 15.1): if a real protection cannot be expressed here,
//! the gap gets fixed BEFORE writing runtime.

pub mod rule;
pub mod schema;

use keel_core::event::Event;
use keel_core::{Decision, OriginClass, Verdict};
use serde::{Deserialize, Serialize};

pub use rule::{
    Branch, Enforcement, Load, Merge, OnFail, Precondition, Report, RuleSpec, Scope, ToolCall,
    ToolRef, When, WhenCondition,
};

/// apiVersion supported by this binary.
pub const API_VERSION: &str = "keel/v1alpha1";

/// `skip_serializing_if` helper for booleans that default to `false`: a flag
/// that is off round-trips as absent, not as an explicit `false`.
pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}

/// Common envelope metadata (spec section 11.1).
///
/// `author`, `adr_ref` and `review_after` are optional HERE because the
/// envelope is common to all kinds — but the JSON Schema for `kind: Rule`
/// requires them (ADR-023): a rule without an origin decision is a rule that,
/// two years from now, enforces something whose rationale nobody remembers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, rename = "adrRef", skip_serializing_if = "Option::is_none")]
    pub adr_ref: Option<String>,
    /// ISO-8601 review window (e.g. "P6M"). Feeds `keel prune` (section 7.7).
    #[serde(
        default,
        rename = "reviewAfter",
        skip_serializing_if = "Option::is_none"
    )]
    pub review_after: Option<String>,
}

/// An Keel document, discriminated by `kind`.
/// Parsed authoring document. Each variant is boxed so the enum stays small and
/// uniform regardless of how big a single spec grows (a Rule spec dwarfs a
/// Workspace one); the enum is a short-lived parse result immediately drained
/// into per-kind vectors by the workspace loader.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Document {
    Workspace(Box<WorkspaceDoc>),
    Rule(Box<RuleDoc>),
    Tool(Box<ToolDoc>),
    Skill(Box<SkillDoc>),
    Agent(Box<AgentDoc>),
    Containment(Box<ContainmentDoc>),
    RuleTest(Box<RuleTestDoc>),
    RepositoryRegistry(Box<RepositoryRegistryDoc>),
    Profile(Box<ProfileDoc>),
    Exception(Box<ExceptionDoc>),
    Blueprint(Box<GovernedComponentDoc>),
    Knowledge(Box<GovernedComponentDoc>),
    Workflow(Box<GovernedComponentDoc>),
    Contract(Box<GovernedComponentDoc>),
    Hook(Box<GovernedComponentDoc>),
    MCPProvider(Box<GovernedComponentDoc>),
    ModelExecutor(Box<GovernedComponentDoc>),
    AgentRoutingPolicy(Box<GovernedComponentDoc>),
    Policy(Box<GovernedComponentDoc>),
}

impl Document {
    pub fn metadata(&self) -> &Metadata {
        match self {
            Document::Workspace(d) => &d.metadata,
            Document::Rule(d) => &d.metadata,
            Document::Tool(d) => &d.metadata,
            Document::Skill(d) => &d.metadata,
            Document::Agent(d) => &d.metadata,
            Document::Containment(d) => &d.metadata,
            Document::RuleTest(d) => &d.metadata,
            Document::RepositoryRegistry(d) => &d.metadata,
            Document::Profile(d) => &d.metadata,
            Document::Exception(d) => &d.metadata,
            Document::Blueprint(d)
            | Document::Knowledge(d)
            | Document::Workflow(d)
            | Document::Contract(d)
            | Document::Hook(d)
            | Document::MCPProvider(d)
            | Document::ModelExecutor(d)
            | Document::AgentRoutingPolicy(d)
            | Document::Policy(d) => &d.metadata,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Document::Workspace(_) => "Workspace",
            Document::Rule(_) => "Rule",
            Document::Tool(_) => "Tool",
            Document::Skill(_) => "Skill",
            Document::Agent(_) => "Agent",
            Document::Containment(_) => "Containment",
            Document::RuleTest(_) => "RuleTest",
            Document::RepositoryRegistry(_) => "RepositoryRegistry",
            Document::Profile(_) => "Profile",
            Document::Exception(_) => "Exception",
            Document::Blueprint(_) => "Blueprint",
            Document::Knowledge(_) => "Knowledge",
            Document::Workflow(_) => "Workflow",
            Document::Contract(_) => "Contract",
            Document::Hook(_) => "Hook",
            Document::MCPProvider(_) => "MCPProvider",
            Document::ModelExecutor(_) => "ModelExecutor",
            Document::AgentRoutingPolicy(_) => "AgentRoutingPolicy",
            Document::Policy(_) => "Policy",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedComponentDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: GovernedComponentSpec,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedComponentSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<ComponentRequirement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentRequirement {
    pub component: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<String>,
    #[serde(default = "required_by_default")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn required_by_default() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    #[serde(default)]
    pub spec: WorkspaceSpec,
}

/// The OS-sandbox backstop (spec section 5.2 runner). Compiles into the
/// snapshot (drift-detectable via the lock) and generates the platform
/// sandbox profile. The hard ring only: file deletions by glob, writes
/// outside the workspace, network — everything the kernel can enforce
/// regardless of PATH. `tool:`-validated rules stay in the shim broker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainmentDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    #[serde(default)]
    pub spec: ContainmentSpec,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainmentSpec {
    #[serde(rename = "denyUnlink", default, skip_serializing_if = "Vec::is_empty")]
    pub deny_unlink: Vec<String>,
    #[serde(
        rename = "denyWriteOutside",
        default,
        skip_serializing_if = "crate::is_false"
    )]
    pub deny_write_outside: bool,
    #[serde(
        rename = "denyNetwork",
        default,
        skip_serializing_if = "crate::is_false"
    )]
    pub deny_network: bool,
}

/// Phase 0 minimal workspace: convention over configuration — rules live in
/// `rules/`, tools in `tools/`, tests in `tests/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: RuleSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: ToolSpec,
}

/// Manifest of an EXTERNAL tool (spec section 4.4: the tool is code — a registered
/// program with a manifest, versioned like any component).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    /// Program + arguments. The event is delivered as JSON via stdin.
    pub command: Vec<String>,
    /// Timeout in milliseconds; expiring yields `unknown`, never a crash.
    #[serde(default, rename = "timeoutMs", skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// How to interpret the tool's output.
    #[serde(default)]
    pub output: ToolOutputKind,
}

/// Supported output formats for external tools.
///
/// SARIF is the normative findings format (ADR-016); `verdict-json` is the
/// minimal three-state contract for in-house scripts; `exit-code` allows
/// wrapping binaries that emit no report (0=valid, 1=invalid, other=unknown).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolOutputKind {
    Sarif,
    #[default]
    VerdictJson,
    ExitCode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: SkillSpec,
}

/// Operational knowledge with loading levels (spec section 14.12): `compact.md` is
/// delivered on first activation; `full.md` on oscillation (section 6.5). The
/// rejected/accepted `examples` feed the packet `exemplar` (section 10.4) — a block
/// whose message is open to interpretation reproduces the failure mode the
/// system exists to prevent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSpec {
    /// Workspace-relative path to the compact variant.
    pub compact: String,
    /// Workspace-relative path to the full variant (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<ExemplarPair>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExemplarPair {
    pub rejected: String,
    pub accepted: String,
}

/// kind: Agent — a logical responsibility routed to a governed ModelExecutor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: AgentSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub role: String,
    pub executor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(
        default,
        rename = "outputSchema",
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<AgentBudget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentBudget {
    #[serde(default, rename = "timeoutMs", skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, rename = "maxTokens", skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleTestDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: RuleTestSpec,
}

/// Declarative fixture: input event → expected result (plan section tests).
/// It is Phase 0a's FUNCTIONAL EQUIVALENCE criterion (spec section 15.1): every gate
/// already deployed is expressed in the DSL and verified case by case against
/// the behavior of the original script.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleTestSpec {
    /// `rule:<id>` — the rule under test.
    pub target: String,
    pub event: Event,
    pub expect: Expectation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expectation {
    /// `fired: false` asserts that the rule does NOT evaluate this event
    /// (scope/detect do not apply). If false, the remaining fields are ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fired: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    /// Expected DECLARED decision (what the rule would ask for; Phase 0's
    /// passive mode records it but forces the effective one to `review`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<Decision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginClass>,
}

/// kind: RepositoryRegistry (spec section 8.5) — links repository identities to
/// Keel projects. This is the input to resolution by repo identity (section 7.1):
/// which project (and therefore which composition chain) a checked-out repo
/// resolves to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryRegistryDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: RepositoryRegistrySpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryRegistrySpec {
    pub repositories: Vec<RepositoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryEntry {
    /// Source-control provider, e.g. `github`.
    pub provider: String,
    /// Provider-scoped identity, e.g. `NuiMarkets/con-app`.
    pub id: String,
    /// Keel project this repository binds to, e.g. `project:nui/con-app`.
    pub project: String,
    /// A locked mapping cannot be reassigned by a lower layer (section 7.1).
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
}

/// kind: Profile (spec section 8.5) — personal preferences. A profile is the
/// LOWEST authority layer: it may select an alternative binding only where an
/// ancestor declares it `overridable`, and it can NEVER weaken a `locked` rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: ProfileSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// Free-form personal preferences (e.g. implementationStrategy, verbosity).
    /// Non-authoritative: kept for round-trip, they govern no action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferences: Option<serde_json::Value>,
}

/// kind: Exception (spec section 7.4, ADR-014) — the ONLY legitimate route to
/// relax a `locked` rule in a concrete context. It is NOT composition: it is an
/// explicit object owned at the scope that declared the lock, with a reason, a
/// bounded scope and an expiry, recorded in the ledger as a `human` decision.
/// Silent weakenings do not exist.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExceptionDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: ExceptionSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExceptionSpec {
    /// `rule:<id>` — the locked rule this exception relaxes.
    pub rule: String,
    /// The scope that declared the lock and therefore owns the exception
    /// (e.g. `organization:nui`).
    pub owner: String,
    pub reason: String,
    /// BOUNDED scope the relaxation applies to (section 7.4: "reason, bounded
    /// scope and expiry"). Mandatory. The compiler carves this coverage OUT of
    /// the locked rule (the rule is lifted within `scope.paths.include`); the
    /// lock stands at full strength everywhere else. An exception with no
    /// `paths.include` cannot bound the relaxation and is rejected — an
    /// unbounded waiver is the silent weakening the mechanism exists to forbid.
    pub scope: Scope,
    /// ISO-8601 expiry date (e.g. `2026-12-31`). An expired exception no longer
    /// suppresses the monotonicity check.
    pub expiry: String,
}

/// DSL loading/validation errors.
#[derive(Debug, thiserror::Error)]
pub enum DslError {
    #[error("invalid YAML: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),
    #[error("document without `kind`")]
    MissingKind,
    #[error("unsupported kind: {0}")]
    UnsupportedKind(String),
    #[error("unsupported apiVersion: {0} (expected {API_VERSION})")]
    UnsupportedApiVersion(String),
    #[error("schema violations in `{kind}` `{id}`:\n{violations}")]
    Schema {
        kind: String,
        id: String,
        violations: String,
    },
}

/// Parses ALL documents from a YAML string (supports multi-doc `---`),
/// validating each one against its JSON Schema BEFORE typing it.
///
/// The schema-first order matters: a schema error points at the exact field
/// in the author's YAML; a serde error would point at the internal
/// representation.
pub fn parse_documents(source: &str) -> Result<Vec<Document>, DslError> {
    let mut docs = Vec::new();
    for raw in serde_yaml_ng::Deserializer::from_str(source) {
        let value = serde_json::Value::deserialize(raw)?;
        if value.is_null() {
            continue; // empty document between separators
        }
        docs.push(parse_value(value)?);
    }
    Ok(docs)
}

fn parse_value(value: serde_json::Value) -> Result<Document, DslError> {
    let kind = value
        .get("kind")
        .and_then(|k| k.as_str())
        .ok_or(DslError::MissingKind)?
        .to_string();

    if let Some(api) = value.get("apiVersion").and_then(|v| v.as_str())
        && api != API_VERSION
    {
        return Err(DslError::UnsupportedApiVersion(api.to_string()));
    }

    // 1) Schema first (Phase 0a deliverable; requires author/adrRef/reviewAfter
    //    on Rule per ADR-023).
    schema::validate(&kind, &value)?;

    // 2) Typing.
    let doc: Document = serde_json::from_value(value)?;
    Ok(doc)
}

impl From<serde_json::Error> for DslError {
    fn from(e: serde_json::Error) -> Self {
        // Can only happen after passing the schema; we report it as a schema
        // error so as not to expose details of the internal representation.
        DslError::Schema {
            kind: "?".into(),
            id: "?".into(),
            violations: e.to_string(),
        }
    }
}
