// SPDX-License-Identifier: Apache-2.0
//! Runtime Snapshot — the effective, immutable, versioned configuration of
//! a session (spec section 6.1, section 10).
//!
//! BOUNDARY RULE: this module does NOT import `keel_dsl`. The snapshot is
//! the COMPILED artifact; the authoring vocabulary stays in the compiler.
//! The price is a small duplication of types (e.g. `ToolRef`); the payoff
//! is that the snapshot hash does not change when the DSL grammar changes
//! (invariant 9) and that the runtime has NO type with which to represent
//! authoring configuration (ADR-004, structural guarantee).

use keel_core::event::EventKind;
use keel_core::{ContentHash, Decision, Reversibility};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Immutable snapshot. No public mutators: it is built once in the compiler
/// and shared read-only (invariant 16 enforced through types).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Canonical hash of the CONTENT (rules+tools+index). `created_at` stays
    /// out of the hash: same configuration → same hash at any time and on
    /// any platform (invariant 9).
    pub hash: ContentHash,
    pub created_at: String,
    pub rules: Vec<CompiledRule>,
    /// External tool manifests, by id.
    pub tools: BTreeMap<String, ExternalToolDef>,
    /// Compiled skills, by id (spec section 14.12). Paths only — the .md content is
    /// read at delivery time so the snapshot hash stays machine-independent
    /// and content edits do not require recompiling.
    #[serde(default)]
    pub skills: BTreeMap<String, CompiledSkill>,
    /// Compiled agents, by id (spec section 14). Part of the governed config, so
    /// they are hashed: a change of executor/model is drift `keel lock --verify`
    /// and `keel ci resolve` must detect (invariant 14).
    #[serde(default)]
    pub agents: BTreeMap<String, CompiledAgent>,
    /// Open-ended Keel-owned resources such as blueprints, knowledge,
    /// workflows, contracts, hooks, providers, policies and model executors.
    #[serde(default)]
    pub components: BTreeMap<String, CompiledComponent>,
    /// OS-sandbox backstop (section 5.2 runner). Part of the governed config,
    /// so it is hashed: a change to what the kernel must deny is drift that
    /// `keel lock --verify` detects. `None` when the workspace declares no
    /// containment — skipped from the hash so existing snapshots are stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub containment: Option<CompiledContainment>,
    /// Event → candidate rules index (spec section 10.1 "Index generation").
    pub index: BTreeMap<EventKind, Vec<usize>>,
}

/// The compiled OS-sandbox backstop. Composed by union across layers
/// (restrictions only add — the same monotonicity philosophy as `locked`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledContainment {
    /// Globs whose matching files the child may not delete (file-write-unlink).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_unlink: Vec<String>,
    /// Deny writes outside the workspace root.
    #[serde(default, skip_serializing_if = "crate::snapshot::is_false")]
    pub deny_write_outside: bool,
    /// Deny outbound network for the child and its descendants.
    #[serde(default, skip_serializing_if = "crate::snapshot::is_false")]
    pub deny_network: bool,
}

pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}

/// The hashable part of the snapshot. It exists as a separate type so that
/// it is EXPLICIT what goes into the hash and what does not.
#[derive(Serialize)]
struct HashableContent<'a> {
    rules: &'a Vec<CompiledRule>,
    tools: &'a BTreeMap<String, ExternalToolDef>,
    skills: &'a BTreeMap<String, CompiledSkill>,
    agents: &'a BTreeMap<String, CompiledAgent>,
    components: &'a BTreeMap<String, CompiledComponent>,
    // Skipped when None so snapshots authored before containment existed keep
    // their exact hash; present → the containment is part of the identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    containment: &'a Option<CompiledContainment>,
    index: &'a BTreeMap<EventKind, Vec<usize>>,
}

impl Snapshot {
    /// Builds the snapshot, computing its canonical hash. The only way to
    /// construct one — there is no way to forge a snapshot with a foreign
    /// hash.
    /// Convenience builder with no agents (used by focused tests).
    pub fn build(
        rules: Vec<CompiledRule>,
        tools: BTreeMap<String, ExternalToolDef>,
        skills: BTreeMap<String, CompiledSkill>,
        created_at: String,
    ) -> Result<Self, serde_json::Error> {
        Self::build_full(rules, tools, skills, BTreeMap::new(), created_at)
    }

    /// Full builder: governed agents go into the canonical hash.
    pub fn build_full(
        rules: Vec<CompiledRule>,
        tools: BTreeMap<String, ExternalToolDef>,
        skills: BTreeMap<String, CompiledSkill>,
        agents: BTreeMap<String, CompiledAgent>,
        created_at: String,
    ) -> Result<Self, serde_json::Error> {
        Self::build_with_components(rules, tools, skills, agents, BTreeMap::new(), created_at)
    }

    pub fn build_with_components(
        rules: Vec<CompiledRule>,
        tools: BTreeMap<String, ExternalToolDef>,
        skills: BTreeMap<String, CompiledSkill>,
        agents: BTreeMap<String, CompiledAgent>,
        components: BTreeMap<String, CompiledComponent>,
        created_at: String,
    ) -> Result<Self, serde_json::Error> {
        let mut index: BTreeMap<EventKind, Vec<usize>> = BTreeMap::new();
        for (i, rule) in rules.iter().enumerate() {
            for kind in &rule.on {
                index.entry(*kind).or_default().push(i);
            }
        }
        let containment = None;
        let hash = ContentHash::of_canonical(&HashableContent {
            rules: &rules,
            tools: &tools,
            skills: &skills,
            agents: &agents,
            components: &components,
            containment: &containment,
            index: &index,
        })?;
        Ok(Snapshot {
            hash,
            created_at,
            rules,
            tools,
            skills,
            agents,
            components,
            containment,
            index,
        })
    }

    /// Attaches the OS-sandbox backstop and RECOMPUTES the canonical hash so
    /// the containment is part of the snapshot identity (drift-detectable via
    /// the lock). The compiler calls this after `build_with_components`.
    pub fn with_containment(
        mut self,
        containment: Option<CompiledContainment>,
    ) -> Result<Self, serde_json::Error> {
        self.containment = containment;
        self.hash = ContentHash::of_canonical(&HashableContent {
            rules: &self.rules,
            tools: &self.tools,
            skills: &self.skills,
            agents: &self.agents,
            components: &self.components,
            containment: &self.containment,
            index: &self.index,
        })?;
        Ok(self)
    }

    /// Candidate rules for an event.
    pub fn candidates(&self, kind: EventKind) -> &[usize] {
        self.index.get(&kind).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn rule_by_id(&self, id: &str) -> Option<&CompiledRule> {
        self.rules.iter().find(|r| r.id == id)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
    }

    /// Loads and VERIFIES: the stored hash must match the one recomputed
    /// from the content. A tampered snapshot does not load (local
    /// self-verification; the strong guarantee lives in the compliance
    /// plane, section 5.1).
    pub fn load(path: &Path) -> Result<Self, SnapshotError> {
        let raw = std::fs::read_to_string(path)?;
        let snap: Snapshot = serde_json::from_str(&raw)?;
        let recomputed = ContentHash::of_canonical(&HashableContent {
            rules: &snap.rules,
            tools: &snap.tools,
            skills: &snap.skills,
            agents: &snap.agents,
            components: &snap.components,
            containment: &snap.containment,
            index: &snap.index,
        })?;
        if recomputed != snap.hash {
            return Err(SnapshotError::HashMismatch {
                stored: snap.hash.to_string(),
                recomputed: recomputed.to_string(),
            });
        }
        Ok(snap)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("could not read the snapshot: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt snapshot: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "snapshot hash mismatch (stored {stored}, recomputed {recomputed}): snapshot tampered with or corrupt"
    )]
    HashMismatch { stored: String, recomputed: String },
}

/// Compiled rule. COMPILED mirror of the authoring Rule: refs resolved,
/// decisions normalized (floor section 4.7 applied), provenance preserved for
/// `explain` and `prune`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledRule {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    // Provenance (ADR-023) — feeds `keel explain` and `keel prune`.
    pub author: String,
    pub adr_ref: String,
    /// ISO-8601 review window as declared ("P6M").
    pub review_after: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reversibility: Option<Reversibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<CompiledScope>,
    pub on: Vec<EventKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<CompiledWhen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detect: Option<CompiledToolCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preconditions: Vec<CompiledPrecondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validate: Option<CompiledToolCall>,
    pub enforcement: CompiledEnforcement,
    /// Environment constraints (section 11.4): `allow`/`deny` evaluated at runtime
    /// against the event's connection context (see `runtime::env_violation`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<CompiledConstraints>,
    /// Composition provenance (section 7.4): the layer whose definition became
    /// effective after composing the chain (e.g. `global`, `project:con-app`).
    /// `None` on a rule that never went through layered composition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_layer: Option<String>,
    /// The layer that declared this rule `locked`, if any — the ancestor the
    /// monotonicity check protected (section 7.4). Feeds `explain` and makes the
    /// composed snapshot self-describing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked_at: Option<String>,
}

/// Compiled `constraints` block (section 11.4). Typed at compile time so a
/// malformed shape fails the build instead of silently doing nothing at eval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledConstraints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvConstraint>,
}

/// Environment allow/deny (section 11.4). `deny` blocks ALWAYS; a non-empty
/// `allow` is a strict allowlist — the action must name an allowed environment,
/// classified from the event's connection context (command + content).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvConstraint {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompiledScope {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

impl CompiledScope {
    /// Does the event fall within the scope? Globs are compiled per call:
    /// Phase 0 simplicity — the per-edit latency budget is precisely one of
    /// the metrics 0b measures (spec section 15.1).
    pub fn matches(&self, file: Option<&str>, language: Option<&str>) -> bool {
        if !self.languages.is_empty() {
            let lang = language
                .map(str::to_string)
                .or_else(|| file.and_then(infer_language));
            match lang {
                Some(l) if self.languages.iter().any(|x| x == &l) => {}
                _ => return false,
            }
        }
        if !self.include.is_empty() || !self.exclude.is_empty() {
            let Some(f) = file else {
                // Rule with a path scope and an event without a file: no match.
                return false;
            };
            if !self.include.is_empty() && !glob_any(&self.include, f) {
                return false;
            }
            if glob_any(&self.exclude, f) {
                return false;
            }
        }
        true
    }
}

/// Minimal language inference by extension, for fixtures that do not
/// declare it. Deliberate and small: the CapabilityManager declares it.
fn infer_language(file: &str) -> Option<String> {
    let ext = file.rsplit('.').next()?;
    let lang = match ext {
        "dart" => "dart",
        "php" => "php",
        "py" => "python",
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        _ => return None,
    };
    Some(lang.to_string())
}

pub(crate) fn glob_any(patterns: &[String], path: &str) -> bool {
    let mut builder = globset::GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = globset::Glob::new(p) {
            builder.add(g);
        }
    }
    match builder.build() {
        Ok(set) => set.is_match(path),
        Err(_) => false,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompiledWhen {
    /// Glob lists from `when.any: files.touch` — one matching is enough.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any_files_touch: Vec<Vec<String>>,
    /// Glob lists from `when.all` — all of them must match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all_files_touch: Vec<Vec<String>>,
}

impl CompiledWhen {
    pub fn matches(&self, files: &[String]) -> bool {
        let touch = |globs: &Vec<String>| files.iter().any(|f| glob_any(globs, f));
        let any_ok = self.any_files_touch.is_empty() || self.any_files_touch.iter().any(touch);
        let all_ok = self.all_files_touch.iter().all(touch);
        any_ok && all_ok
    }
}

/// Compiled tool reference. Deliberate duplication with respect to the DSL:
/// the price of the snapshot ⇏ dsl boundary (see module header).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub enum CompiledToolRef {
    Builtin(String),
    External(String),
}

impl std::fmt::Display for CompiledToolRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompiledToolRef::Builtin(id) => write!(f, "builtin:{id}"),
            CompiledToolRef::External(id) => write!(f, "tool:{id}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledToolCall {
    pub using: CompiledToolRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledPrecondition {
    pub using: CompiledToolRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<serde_json::Value>,
    /// Decision declared for failure, already mapped onto the lattice by the
    /// compiler (deny→block: a failed precondition is certainty of violation,
    /// not uncertainty — see keel-dsl::OnFail).
    pub on_fail_declared: Decision,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompiledEnforcement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid: Option<CompiledBranch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown: Option<CompiledBranch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid: Option<CompiledBranch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub always: Option<CompiledBranch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledBranch {
    pub decision: Decision,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load_skills: Vec<String>,
    /// Forward-declared context-economy capabilities (spec section 11.3, future section 9).
    /// Recorded in the evidence for visibility, but the runtime does not yet
    /// ACTIVATE/limit them — that is Phase 2. Kept (not dropped) so the DSL
    /// stays aligned with the reference example; surfaced in `branch_detail`
    /// so it is a declared field, never a silent no-op.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_message: Option<String>,
    /// Agent to invoke on this branch. In Phase 0 it is RECORDED in the
    /// evidence and NEVER executed (the semantic evaluator is Phase 2; no
    /// model runs in the passive slice).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoke_agent: Option<String>,
}

/// Compiled manifest of an external tool (spec section 4.4: the tool is code).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalToolDef {
    pub id: String,
    pub command: Vec<String>,
    pub timeout_ms: u64,
    pub output: OutputKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputKind {
    Sarif,
    VerdictJson,
    ExitCode,
}

/// Compiled skill (spec section 14.12): loading levels + exemplar pairs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledSkill {
    pub id: String,
    /// Component version from the authored metadata.
    #[serde(default = "default_component_version")]
    pub version: String,
    /// One-line description for the exposed catalog (D-013). Empty when the
    /// author did not provide one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Workspace-relative path to the compact variant.
    pub compact: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full: Option<String>,
    /// Rejected/accepted pairs feeding the packet `exemplar` (section 10.4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompiledComponent {
    pub kind: String,
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<CompiledRequirement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompiledRequirement {
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<String>,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn default_component_version() -> String {
    "unversioned".to_string()
}

/// Compiled agent (spec section 14): a logical responsibility run by an executor.
/// Part of the governed config and therefore hashed (invariant 14).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledAgent {
    pub id: String,
    pub role: String,
    /// Executor id this agent routes to (the `executor:` prefix is stripped).
    pub executor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    /// Workspace-relative path to the JSON Schema the AgentResult is validated
    /// against (invariant 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
}

#[cfg(test)]
#[path = "../tests-unit/snapshot.rs"]
mod tests;
