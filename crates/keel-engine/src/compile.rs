// SPDX-License-Identifier: Apache-2.0
//! Compiler — the spec section 10.1 pipeline, trimmed down to Phase 0.
//!
//! ```text
//! Parse                → workspace.rs (YAML → Document, schema-validated)
//! Schema validation    → keel-dsl::schema (ADR-023 active)
//! Reference resolution → every tool ref resolves to a builtin or a manifest
//! Composition          → *** DOCUMENTED STUB *** (see below)
//! Conflict detection   → duplicate IDs at the same level (section 7.6)
//! Tool validation      → compilable regexes, detect builtin-only in Phase 0
//! Index generation     → event → candidate rules
//! Snapshot creation    → immutable artifact with a canonical hash
//! ```
//!
//! ── Composition: why it is a no-op in Phase 0 ───────────────────────────
//! The `locked` monotonicity check (spec section 7.4, D1–D4) operates on the
//! COMPOSITION of layers (org → platform → project → team → profile).
//! Phase 0 has ONE workspace and ONE project: there is no second authority
//! layer against which to verify that nothing gets weakened. The step exists
//! here as an explicit function — not silently omitted — and activates when
//! the second layer arrives (Phase 1+). The lattice it will use (D3) already
//! lives in keel-core::Decision.
//!
//! The compiler is a PURE FUNCTION config → snapshot: it does not import the
//! runtime (forbidden edge compiler ⇏ runtime), does not evaluate events,
//! knows nothing about sessions. That is why local and CI, running this same
//! code over the same workspace, produce bit-for-bit the same hash
//! (invariant 9).

use crate::composition::{ComposeLayer, Inheritance, compose};
use crate::snapshot::{
    CompiledAgent, CompiledBranch, CompiledEnforcement, CompiledExecutor, CompiledPrecondition,
    CompiledRule, CompiledScope, CompiledSkill, CompiledToolCall, CompiledToolRef, CompiledWhen,
    ExternalToolDef, OutputKind, Snapshot,
};
use crate::tools::{BUILTIN_DETECTORS, BUILTIN_PRECONDITIONS};
use crate::workspace::WorkspaceFiles;
use keel_core::{Decision, Reversibility};
use keel_dsl::{Branch, Enforcement, OnFail, RuleDoc, ToolDoc, ToolRef};
use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_TOOL_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug)]
pub struct CompileOutcome {
    pub snapshot: Snapshot,
    /// Non-blocking debt (section 6.5: exemplar/report missing on block rules;
    /// section 4.7 floors applied by normalization). The rule ledger starts here:
    /// debt is declared, not swallowed.
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error(
        "duplicate id `{0}`: two components at the same authority level (section 7.6) — resolution required, never silent"
    )]
    DuplicateId(String),
    #[error("rule `{rule}`: unresolvable reference `{reference}` — {hint}")]
    UnresolvedTool {
        rule: String,
        reference: String,
        hint: String,
    },
    #[error(
        "agent `{agent}` routes to executor `{executor}`, which is not declared in agents/ — a governed agent must resolve to a known executor (invariant 11)"
    )]
    UnresolvedExecutor { agent: String, executor: String },
    #[error("rule `{rule}`: invalid regex in detect/validate: {source}")]
    InvalidRegex {
        rule: String,
        #[source]
        source: regex::Error,
    },
    #[error(
        "rule `{rule}`: external detect not supported in Phase 0 (the section 11.4 corpus uses builtin detectors only); use a builtin or move the tool to validate"
    )]
    ExternalDetect { rule: String },
    #[error("rule `{rule}`: reviewAfter `{value}` is not a valid ISO-8601 duration")]
    BadReviewAfter { rule: String, value: String },
    #[error("rule `{rule}`: malformed `constraints` block — {source}")]
    BadConstraints {
        rule: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("snapshot not serializable: {0}")]
    Snapshot(#[from] serde_json::Error),
    #[error(transparent)]
    Monotonicity(#[from] crate::composition::MonotonicityViolation),
}

/// One composition layer to compile: its rules/tools/etc. and the label used in
/// monotonicity reports (`global`, `organization:nui`, `project:con-app`).
pub struct CompileLayer<'a> {
    pub label: String,
    pub files: &'a WorkspaceFiles,
}

/// Compiles a single flat workspace into an immutable snapshot. A flat
/// workspace is one composition layer, so this is `compile_layered` over a
/// one-element chain — the same code path, no special case.
pub fn compile(files: &WorkspaceFiles, created_at: String) -> Result<CompileOutcome, CompileError> {
    compile_layered(
        &[CompileLayer {
            label: "project".to_string(),
            files,
        }],
        created_at,
    )
}

/// Compiles a composition chain (spec section 7.2, highest authority first) into
/// one snapshot: unions the components, compiles each layer's rules, composes
/// them into effective rules while verifying `locked` monotonicity (section 7.4,
/// [`compose`]), then builds the snapshot.
pub fn compile_layered(
    chain: &[CompileLayer],
    created_at: String,
) -> Result<CompileOutcome, CompileError> {
    let mut warnings = Vec::new();
    // Non-rule component ids are unique across the whole chain (section 7.6:
    // there is no defined composition for tools/skills/agents/executors, so a
    // clash is a conflict, never a silent override). Rule ids may recur across
    // layers — that is composition, handled by `compose`.
    let mut seen = BTreeSet::new();

    // ── Tool manifests ──
    let mut tools: BTreeMap<String, ExternalToolDef> = BTreeMap::new();
    for layer in chain {
        for tool in &layer.files.tools {
            if !seen.insert(tool.metadata.id.clone()) {
                return Err(CompileError::DuplicateId(tool.metadata.id.clone()));
            }
            tools.insert(tool.metadata.id.clone(), compile_tool(tool));
        }
    }

    // ── Skill manifests (section 14.12): paths validated, content read at delivery ──
    let mut skills: BTreeMap<String, CompiledSkill> = BTreeMap::new();
    for layer in chain {
        for skill in &layer.files.skills {
            if !seen.insert(skill.metadata.id.clone()) {
                return Err(CompileError::DuplicateId(skill.metadata.id.clone()));
            }
            if !layer.files.root.join(&skill.spec.compact).exists() {
                warnings.push(format!(
                    "skill `{}`: compact file `{}` not found in the workspace — delivery will degrade to a reference",
                    skill.metadata.id, skill.spec.compact
                ));
            }
            skills.insert(
                skill.metadata.id.clone(),
                CompiledSkill {
                    id: skill.metadata.id.clone(),
                    compact: skill.spec.compact.clone(),
                    full: skill.spec.full.clone(),
                    examples: skill
                        .spec
                        .examples
                        .iter()
                        .map(|e| (e.rejected.clone(), e.accepted.clone()))
                        .collect(),
                },
            );
        }
    }

    // ── Agent executors (section 14.1/14.8): how/where an agent runs ──
    let mut executors: BTreeMap<String, CompiledExecutor> = BTreeMap::new();
    for layer in chain {
        for x in &layer.files.executors {
            if !seen.insert(x.metadata.id.clone()) {
                return Err(CompileError::DuplicateId(x.metadata.id.clone()));
            }
            executors.insert(
                x.metadata.id.clone(),
                CompiledExecutor {
                    id: x.metadata.id.clone(),
                    command: x.spec.command.clone(),
                    model: x.spec.model.clone(),
                    timeout_ms: x.spec.timeout_ms,
                    env: x.spec.env.clone(),
                },
            );
        }
    }

    // ── Agents (section 14): resolve the executor reference (invariant 11).
    //    Resolved after ALL executors so an agent may use one from any layer. ──
    let mut agents: BTreeMap<String, CompiledAgent> = BTreeMap::new();
    for layer in chain {
        for a in &layer.files.agents {
            if !seen.insert(a.metadata.id.clone()) {
                return Err(CompileError::DuplicateId(a.metadata.id.clone()));
            }
            let executor_id = a
                .spec
                .executor
                .strip_prefix("executor:")
                .unwrap_or(&a.spec.executor)
                .to_string();
            if !executors.contains_key(&executor_id) {
                return Err(CompileError::UnresolvedExecutor {
                    agent: a.metadata.id.clone(),
                    executor: executor_id,
                });
            }
            agents.insert(
                a.metadata.id.clone(),
                CompiledAgent {
                    id: a.metadata.id.clone(),
                    role: a.spec.role.clone(),
                    executor: executor_id,
                    objective: a.spec.objective.clone(),
                    output_schema: a.spec.output_schema.clone(),
                    timeout_ms: a.spec.budget.as_ref().and_then(|b| b.timeout_ms),
                    max_tokens: a.spec.budget.as_ref().and_then(|b| b.max_tokens),
                },
            );
        }
    }

    // ── Reference resolution + Tool validation + Policy compilation, per layer.
    //    Rules are compiled against the UNIONED tools/skills (a rule may
    //    reference a tool defined in a higher layer). ──
    let mut compose_layers = Vec::with_capacity(chain.len());
    for layer in chain {
        let mut layer_ids = BTreeSet::new();
        let mut layer_rules = Vec::with_capacity(layer.files.rules.len());
        for doc in &layer.files.rules {
            // Two rules with the same id in ONE layer is a same-level conflict
            // (section 7.6); the same id ACROSS layers is composition.
            if !layer_ids.insert(doc.metadata.id.clone()) {
                return Err(CompileError::DuplicateId(doc.metadata.id.clone()));
            }
            // A rule id must not collide with a component id.
            if seen.contains(&doc.metadata.id) {
                return Err(CompileError::DuplicateId(doc.metadata.id.clone()));
            }
            let compiled = compile_rule(doc, &tools, &skills, &mut warnings)?;
            layer_rules.push((inheritance_of(doc), compiled));
        }
        compose_layers.push(ComposeLayer {
            label: layer.label.clone(),
            rules: layer_rules,
        });
    }

    // ── Composition + monotonicity verification (section 7.4) ──
    let rules = compose(&compose_layers)?;

    // ── Index generation + Snapshot creation (canonical hash, invariant 9) ──
    let snapshot = Snapshot::build_full(rules, tools, skills, agents, executors, created_at)?;
    Ok(CompileOutcome { snapshot, warnings })
}

/// The composition inheritance a rule declares (spec section 7.3).
fn inheritance_of(doc: &RuleDoc) -> Inheritance {
    Inheritance {
        locked: doc.spec.locked,
        overridable: doc.spec.overridable,
        merge: doc.spec.merge,
    }
}

fn compile_tool(tool: &ToolDoc) -> ExternalToolDef {
    // Commands are preserved AS AUTHORED (relative paths included): pinning
    // them to absolute paths would put machine paths into the snapshot and
    // its hash would no longer be reproducible across machines (invariant 9
    // / invariant 5). Resolution happens at execution time: the Tool Runner
    // launches the subprocess with cwd = workspace root.
    ExternalToolDef {
        id: tool.metadata.id.clone(),
        command: tool.spec.command.clone(),
        timeout_ms: tool.spec.timeout_ms.unwrap_or(DEFAULT_TOOL_TIMEOUT_MS),
        output: match tool.spec.output {
            keel_dsl::ToolOutputKind::Sarif => OutputKind::Sarif,
            keel_dsl::ToolOutputKind::VerdictJson => OutputKind::VerdictJson,
            keel_dsl::ToolOutputKind::ExitCode => OutputKind::ExitCode,
        },
    }
}

fn compile_rule(
    rule: &RuleDoc,
    tools: &BTreeMap<String, ExternalToolDef>,
    skills: &BTreeMap<String, CompiledSkill>,
    warnings: &mut Vec<String>,
) -> Result<CompiledRule, CompileError> {
    let id = &rule.metadata.id;
    let spec = &rule.spec;

    // reviewAfter validated at compile time: prune (section 7.7) depends on it.
    let review_after = rule.metadata.review_after.clone().unwrap_or_default();
    if review_after.parse::<jiff::Span>().is_err() {
        return Err(CompileError::BadReviewAfter {
            rule: id.clone(),
            value: review_after,
        });
    }

    // ── detect: builtin-only in Phase 0; regex validated here, not at runtime ──
    let detect = match &spec.detect {
        Some(call) => {
            match &call.using {
                ToolRef::External(_) => {
                    return Err(CompileError::ExternalDetect { rule: id.clone() });
                }
                ToolRef::Builtin(bid) => {
                    if !BUILTIN_DETECTORS.contains(&bid.as_str()) {
                        return Err(CompileError::UnresolvedTool {
                            rule: id.clone(),
                            reference: call.using.to_string(),
                            hint: format!("available builtin detectors: {BUILTIN_DETECTORS:?}"),
                        });
                    }
                    validate_regex_param(id, bid, call.with.as_ref())?;
                }
            }
            Some(to_compiled_call(call))
        }
        None => None,
    };

    // ── preconditions: known builtin or tool with a manifest ──
    let mut preconditions = Vec::new();
    for pre in &spec.preconditions {
        match &pre.using {
            ToolRef::Builtin(bid) if !BUILTIN_PRECONDITIONS.contains(&bid.as_str()) => {
                return Err(CompileError::UnresolvedTool {
                    rule: id.clone(),
                    reference: pre.using.to_string(),
                    hint: format!("available builtin preconditions: {BUILTIN_PRECONDITIONS:?}"),
                });
            }
            ToolRef::External(tid) if !tools.contains_key(tid) => {
                return Err(CompileError::UnresolvedTool {
                    rule: id.clone(),
                    reference: pre.using.to_string(),
                    hint: "declare a `kind: Tool` with that id in tools/".into(),
                });
            }
            _ => {}
        }
        preconditions.push(CompiledPrecondition {
            using: to_compiled_ref(&pre.using),
            with: pre.with.clone(),
            // deny → block: certainty of violation, not uncertainty (see
            // keel-dsl::OnFail). The mapping happens HERE so the runtime
            // never knows the authoring vocabulary.
            on_fail_declared: pre.on_fail.as_declared_decision(),
        });
    }

    // ── validate: text builtin or tool with a manifest ──
    let validate = match &spec.validate {
        Some(call) => {
            match &call.using {
                ToolRef::Builtin(bid) => {
                    if !["text.contains", "text.regex"].contains(&bid.as_str()) {
                        return Err(CompileError::UnresolvedTool {
                            rule: id.clone(),
                            reference: call.using.to_string(),
                            hint:
                                "in validate only text.contains/text.regex are allowed as builtins"
                                    .into(),
                        });
                    }
                    validate_regex_param(id, bid, call.with.as_ref())?;
                }
                ToolRef::External(tid) => {
                    if !tools.contains_key(tid) {
                        return Err(CompileError::UnresolvedTool {
                            rule: id.clone(),
                            reference: call.using.to_string(),
                            hint: "declare a `kind: Tool` with that id in tools/".into(),
                        });
                    }
                }
            }
            Some(to_compiled_call(call))
        }
        None => None,
    };

    let enforcement = compile_enforcement(id, &spec.enforcement, spec.reversibility, warnings);

    // ── Reference resolution for load.skills (section 10.1): a skill a rule loads
    //    must exist — a dangling reference would silently deliver nothing,
    //    exactly the omission class this system eliminates.
    for branch in [
        enforcement.invalid.as_ref(),
        enforcement.unknown.as_ref(),
        enforcement.valid.as_ref(),
        enforcement.always.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        for skill_ref in &branch.load_skills {
            let skill_id = skill_ref
                .strip_prefix("skill:")
                .unwrap_or(skill_ref)
                .split('#')
                .next()
                .unwrap_or_default();
            if !skills.contains_key(skill_id) {
                return Err(CompileError::UnresolvedTool {
                    rule: id.clone(),
                    reference: skill_ref.clone(),
                    hint: "declare a `kind: Skill` with that id in skills/".into(),
                });
            }
        }
    }

    // Rule debt (section 6.5/section 10.4): a block with neither message nor skills
    // produces findings open to interpretation — declared as a warning.
    if let Some(inv) = &enforcement.invalid
        && inv.decision >= Decision::Block
        && inv.report_message.is_none()
        && inv.load_skills.is_empty()
    {
        warnings.push(format!(
                "rule `{id}`: invalid branch with decision block but no report.message or load.skills — rule debt (section 6.5): an ambiguous block reproduces the failure mode the system fights"
            ));
    }

    // Rule debt (section 10.4:840): a `block` branch that loads skills but none of
    // them provide an exemplar pair — the mandatory rejected/accepted companion
    // of a block is missing, so the packet can only say "don't", not "do this".
    if let Some(inv) = &enforcement.invalid
        && inv.decision >= Decision::Block
        && !inv.load_skills.is_empty()
        && !inv.load_skills.iter().any(|skill_ref| {
            let sid = skill_ref
                .strip_prefix("skill:")
                .unwrap_or(skill_ref)
                .split('#')
                .next()
                .unwrap_or_default();
            skills.get(sid).is_some_and(|s| !s.examples.is_empty())
        })
    {
        warnings.push(format!(
            "rule `{id}`: block branch loads skills but none provide an exemplar pair — rule debt (section 10.4): a block should carry a rejected/accepted example"
        ));
    }

    // Type the loose DSL `constraints` Value at compile: a malformed shape is a
    // build error, never a silently-ignored field at eval (section 11.4).
    let constraints = match &spec.constraints {
        Some(v) => Some(
            serde_json::from_value::<crate::snapshot::CompiledConstraints>(v.clone()).map_err(
                |e| CompileError::BadConstraints {
                    rule: id.clone(),
                    source: e,
                },
            )?,
        ),
        None => None,
    };

    Ok(CompiledRule {
        id: id.clone(),
        version: rule.metadata.version.clone(),
        author: rule.metadata.author.clone().unwrap_or_default(),
        adr_ref: rule.metadata.adr_ref.clone().unwrap_or_default(),
        review_after,
        reversibility: spec.reversibility,
        scope: spec.scope.as_ref().map(|s| CompiledScope {
            languages: s.languages.clone(),
            include: s
                .paths
                .as_ref()
                .map(|p| p.include.clone())
                .unwrap_or_default(),
            exclude: s
                .paths
                .as_ref()
                .map(|p| p.exclude.clone())
                .unwrap_or_default(),
        }),
        on: spec.on.clone(),
        when: spec.when.as_ref().map(|w| CompiledWhen {
            any_files_touch: w.any.iter().filter_map(|c| c.files_touch.clone()).collect(),
            all_files_touch: w.all.iter().filter_map(|c| c.files_touch.clone()).collect(),
        }),
        detect,
        preconditions,
        validate,
        enforcement,
        constraints,
        // Provenance is stamped by the composition step, which knows the layers.
        origin_layer: None,
        locked_at: None,
    })
}

/// detect/validate regexes are compiled HERE: a broken regex is an author
/// error caught at compile time, not a surprise fail-open at runtime.
fn validate_regex_param(
    rule: &str,
    builtin: &str,
    with: Option<&serde_json::Value>,
) -> Result<(), CompileError> {
    if builtin != "text.regex" {
        return Ok(());
    }
    let pattern = with
        .and_then(|w| w.get("pattern"))
        .and_then(|p| p.as_str())
        .unwrap_or("");
    regex::Regex::new(pattern)
        .map(|_| ())
        .map_err(|source| CompileError::InvalidRegex {
            rule: rule.to_string(),
            source,
        })
}

fn compile_enforcement(
    rule_id: &str,
    e: &Enforcement,
    reversibility: Option<Reversibility>,
    warnings: &mut Vec<String>,
) -> CompiledEnforcement {
    let mut unknown = e.unknown.as_ref().map(to_compiled_branch);

    // Floor section 4.7 normalization (ADR-017): on an IRREVERSIBLE rule the
    // `unknown` branch has `deny-pending-approval` as its floor, regardless
    // of what it composes to. It is normalized at COMPILE time (a static
    // property) so the runtime never has to reason about reversibility.
    if reversibility == Some(Reversibility::Irreversible) {
        let floor = Decision::DenyPendingApproval;
        match &mut unknown {
            Some(branch) if branch.decision < floor => {
                warnings.push(format!(
                    "rule `{rule_id}`: unknown branch raised to deny-pending-approval (floor section 4.7 for irreversible actions: uncertainty escalates to a human, never to a model)"
                ));
                branch.decision = floor;
            }
            None => {
                unknown = Some(CompiledBranch {
                    decision: floor,
                    load_skills: vec![],
                    load_capabilities: vec![],
                    report_message: None,
                    invoke_agent: None,
                });
            }
            _ => {}
        }
    }

    CompiledEnforcement {
        invalid: e.invalid.as_ref().map(to_compiled_branch),
        unknown,
        valid: e.valid.as_ref().map(to_compiled_branch),
        always: e.always.as_ref().map(to_compiled_branch),
    }
}

fn to_compiled_branch(b: &Branch) -> CompiledBranch {
    CompiledBranch {
        decision: b.decision,
        load_skills: b
            .load
            .as_ref()
            .map(|l| l.skills.clone())
            .unwrap_or_default(),
        load_capabilities: b
            .load
            .as_ref()
            .map(|l| l.capabilities.clone())
            .unwrap_or_default(),
        report_message: b.report.as_ref().and_then(|r| r.message.clone()),
        invoke_agent: b.invoke.as_ref().map(|i| i.agent.clone()),
    }
}

fn to_compiled_call(call: &keel_dsl::ToolCall) -> CompiledToolCall {
    CompiledToolCall {
        using: to_compiled_ref(&call.using),
        with: call.with.clone(),
    }
}

fn to_compiled_ref(r: &ToolRef) -> CompiledToolRef {
    match r {
        ToolRef::Builtin(id) => CompiledToolRef::Builtin(id.clone()),
        ToolRef::External(id) => CompiledToolRef::External(id.clone()),
    }
}

// Compat: OnFail comes from keel-dsl; its mapping onto the lattice is tested there.
use keel_dsl as _;
const _: fn(OnFail) -> Decision = OnFail::as_declared_decision;

#[cfg(test)]
#[path = "../tests-unit/compile.rs"]
mod tests;
