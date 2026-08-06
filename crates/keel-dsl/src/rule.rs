// SPDX-License-Identifier: Apache-2.0
//! `kind: Rule` — the declarative anatomy of a rule (spec section 11.3–11.5).
//!
//! The anatomy does not change across languages; the tool changes (spec section 11.4).
//! These types must be able to express the real gates of the section 11.4 corpus
//! WITHOUT LOSS — that is the Phase 0a test. If a gate does not fit here, the
//! gap gets fixed in the DSL before writing more runtime.

use keel_core::event::EventKind;
use keel_core::{Decision, Reversibility};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSpec {
    /// Decides the fate of `unknown` (section 4.7). Optional because cognitive
    /// activation rules (enforcement.always) do not govern an action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reversibility: Option<Reversibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    /// Events that trigger the rule (section 11.2).
    pub on: Vec<EventKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<When>,
    /// Economic prefilter. It NEVER decides (section 4.5): a match only opens the
    /// door to `validate`. A detector false positive costs microseconds of
    /// tool CPU; never a blocked action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detect: Option<ToolCall>,
    /// Conditions on the STATE OF THE WORLD at the moment of the request
    /// (ADR-022): live credential, flag present, explicit env. A distinct
    /// category from `validate` (which evaluates the content of the action).
    /// They are evaluated first, in order, each with its own `onFail`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preconditions: Vec<Precondition>,
    /// The real verdict (AST, parser, static analysis — 0 tokens).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validate: Option<ToolCall>,
    pub enforcement: Enforcement,
    /// Additional constraints (e.g. `environment.allow/deny` from the section 11.4
    /// SQL gate). Phase 0 preserves and records them; their active evaluation
    /// arrives with enforcement (Phase 1). Keeping them loose here is
    /// deliberate: losing them in the round-trip would violate the Phase 0a
    /// criterion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<Paths>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paths {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

/// Additional activation condition (section 11.4 corpus: cognitive activation).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct When {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any: Vec<WhenCondition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all: Vec<WhenCondition>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhenCondition {
    /// `files.touch: [globs]` — the task touches matching files.
    #[serde(
        default,
        rename = "files.touch",
        skip_serializing_if = "Option::is_none"
    )]
    pub files_touch: Option<Vec<String>>,
}

/// Reference to a tool: `builtin:<id>` or `tool:<id>` (section 4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRef {
    Builtin(String),
    External(String),
}

impl FromStr for ToolRef {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(id) = s.strip_prefix("builtin:") {
            Ok(ToolRef::Builtin(id.to_string()))
        } else if let Some(id) = s.strip_prefix("tool:") {
            Ok(ToolRef::External(id.to_string()))
        } else {
            Err(format!(
                "invalid tool reference `{s}`: expected `builtin:<id>` or `tool:<id>`"
            ))
        }
    }
}

impl fmt::Display for ToolRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolRef::Builtin(id) => write!(f, "builtin:{id}"),
            ToolRef::External(id) => write!(f, "tool:{id}"),
        }
    }
}

impl Serialize for ToolRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ToolRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Invocation of a tool with its parameters (`detect:` and `validate:`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub using: ToolRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<serde_json::Value>,
    /// Requested inputs (file, diff, projectContext, callGraphSlice…).
    /// Phase 0 preserves them; the runtime delivers the full event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Precondition {
    pub using: ToolRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<serde_json::Value>,
    #[serde(rename = "onFail")]
    pub on_fail: OnFail,
}

/// Consequence of a failed precondition.
///
/// `deny` is a CERTAIN DENIAL (not uncertainty): it maps to declared decision
/// `block` with verdict `invalid`. `deny-pending-approval` is reserved for the
/// `unknown` branch of irreversible actions (section 4.7) — uncertainty escalates to
/// a human; certainty of violation blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnFail {
    Deny,
    Block,
    Review,
}

impl OnFail {
    /// Equivalent declared decision in the lattice (section 7.4-D3).
    pub fn as_declared_decision(self) -> Decision {
        match self {
            OnFail::Deny | OnFail::Block => Decision::Block,
            OnFail::Review => Decision::Review,
        }
    }
}

/// Enforcement branches per verdict (section 11.3). `always` is the cognitive
/// activation variant (loads context without governing an action).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enforcement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid: Option<Branch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown: Option<Branch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid: Option<Branch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub always: Option<Branch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Branch {
    pub decision: Decision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load: Option<Load>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<Report>,
    /// Agent invocation on the `unknown` branch (section 11.4). In Phase 0 it is
    /// PARSED and RECORDED, never executed: the semantic evaluator is Phase 2
    /// and no model runs in the passive slice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoke: Option<Invoke>,
}

/// Cognitive load associated with a branch (section 7.4-D4: under `locked`
/// composition, extend-only — never replaceable by poorer variants).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Load {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    /// Context-economy capabilities to activate for the agent (spec section 11.3,
    /// future section 9). Forward-declared: compiled into the snapshot and surfaced in
    /// the evidence, but the runtime does not yet activate/limit them (Phase 2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// Finding schema — SARIF is the normative one (ADR-016).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invoke {
    pub agent: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
}

#[cfg(test)]
#[path = "../tests-unit/rule.rs"]
mod tests;
