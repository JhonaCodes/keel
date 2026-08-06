// SPDX-License-Identifier: Apache-2.0
//! JSON-Schema validation of the raw YAML, BEFORE typing.
//!
//! WHY SCHEMA-FIRST: the schema is the DELIVERABLE of Phase 0a (spec
//! section 15.1) — the mechanical proof that the DSL expresses the real gates
//! without losing anything — and its errors point at the exact field in the
//! author's YAML (`/metadata/adrRef`), not at serde's internal
//! representation. This is where ADR-023 is enforced: a Rule without
//! `author`/`adrRef`/`reviewAfter` does not get past this function.
//!
//! The schemas live versioned in `schemas/` (repo root) and are embedded in
//! the binary: a single executable behaves the same locally and in CI.

use crate::DslError;
use jsonschema::Validator;
use std::sync::OnceLock;

const RULE_SCHEMA: &str = include_str!("../../../schemas/rule.schema.json");
const TOOL_SCHEMA: &str = include_str!("../../../schemas/tool.schema.json");
const RULETEST_SCHEMA: &str = include_str!("../../../schemas/ruletest.schema.json");
const SKILL_SCHEMA: &str = include_str!("../../../schemas/skill.schema.json");
const AGENT_SCHEMA: &str = include_str!("../../../schemas/agent.schema.json");
const AGENTEXECUTOR_SCHEMA: &str = include_str!("../../../schemas/agentexecutor.schema.json");
const WORKSPACE_SCHEMA: &str = include_str!("../../../schemas/workspace.schema.json");

fn validator_for(kind: &str) -> Option<&'static Validator> {
    static RULE: OnceLock<Validator> = OnceLock::new();
    static TOOL: OnceLock<Validator> = OnceLock::new();
    static RULETEST: OnceLock<Validator> = OnceLock::new();
    static WORKSPACE: OnceLock<Validator> = OnceLock::new();
    static SKILL: OnceLock<Validator> = OnceLock::new();
    static AGENT: OnceLock<Validator> = OnceLock::new();
    static AGENTEXEC: OnceLock<Validator> = OnceLock::new();

    fn build(raw: &str) -> Validator {
        let schema: serde_json::Value =
            serde_json::from_str(raw).expect("embedded schema is invalid: build bug");
        jsonschema::validator_for(&schema).expect("embedded schema does not compile: build bug")
    }

    match kind {
        "Rule" => Some(RULE.get_or_init(|| build(RULE_SCHEMA))),
        "Tool" => Some(TOOL.get_or_init(|| build(TOOL_SCHEMA))),
        "RuleTest" => Some(RULETEST.get_or_init(|| build(RULETEST_SCHEMA))),
        "Workspace" => Some(WORKSPACE.get_or_init(|| build(WORKSPACE_SCHEMA))),
        "Skill" => Some(SKILL.get_or_init(|| build(SKILL_SCHEMA))),
        "Agent" => Some(AGENT.get_or_init(|| build(AGENT_SCHEMA))),
        "AgentExecutor" => Some(AGENTEXEC.get_or_init(|| build(AGENTEXECUTOR_SCHEMA))),
        _ => None,
    }
}

/// Validates a raw document against the schema for its `kind`.
///
/// Reports ALL violations at once (not just the first one): the author of a
/// rule must be able to fix the whole file in one pass.
pub fn validate(kind: &str, value: &serde_json::Value) -> Result<(), DslError> {
    let validator =
        validator_for(kind).ok_or_else(|| DslError::UnsupportedKind(kind.to_string()))?;

    let violations: Vec<String> = validator
        .iter_errors(value)
        .map(|e| format!("  {}: {}", e.instance_path, e))
        .collect();

    if violations.is_empty() {
        return Ok(());
    }

    let id = value
        .pointer("/metadata/id")
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string();

    Err(DslError::Schema {
        kind: kind.to_string(),
        id,
        violations: violations.join("\n"),
    })
}

#[cfg(test)]
#[path = "../tests-unit/schema.rs"]
mod tests;