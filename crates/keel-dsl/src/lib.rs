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
    Branch, Enforcement, Load, OnFail, Precondition, Report, RuleSpec, Scope, ToolCall, ToolRef,
    When, WhenCondition,
};

/// apiVersion supported by this binary.
pub const API_VERSION: &str = "keel/v1alpha1";

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
    AgentExecutor(Box<AgentExecutorDoc>),
    RuleTest(Box<RuleTestDoc>),
}

impl Document {
    pub fn metadata(&self) -> &Metadata {
        match self {
            Document::Workspace(d) => &d.metadata,
            Document::Rule(d) => &d.metadata,
            Document::Tool(d) => &d.metadata,
            Document::Skill(d) => &d.metadata,
            Document::Agent(d) => &d.metadata,
            Document::AgentExecutor(d) => &d.metadata,
            Document::RuleTest(d) => &d.metadata,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Document::Workspace(_) => "Workspace",
            Document::Rule(_) => "Rule",
            Document::Tool(_) => "Tool",
            Document::Skill(_) => "Skill",
            Document::Agent(_) => "Agent",
            Document::AgentExecutor(_) => "AgentExecutor",
            Document::RuleTest(_) => "RuleTest",
        }
    }
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

/// kind: Agent (spec section 14) — a logical responsibility executed by an
/// AgentExecutor. Minimal Phase-2-seed shape.
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

/// kind: AgentExecutor (spec section 14.1/section 14.8) — how/where an Agent runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutorDoc {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: AgentExecutorSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutorSpec {
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, rename = "timeoutMs", skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Environment allowlist (section 13.1: "no secret inheritance to child sessions
    /// by default"). The executor subprocess receives ONLY these host env vars
    /// (plus PATH, needed to resolve the program) — everything else is scrubbed.
    /// Empty (the default) = no inheritance: an executor that needs a credential
    /// must name it here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
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
