// SPDX-License-Identifier: Apache-2.0
//! Compiler — the spec section 10.1 pipeline, trimmed down to Phase 0.
//!
//! ```text
//! Parse                → workspace.rs (YAML → Document, schema-validated)
//! Schema validation    → keel-dsl::schema (ADR-023 active)
//! Reference resolution → every tool ref resolves to a builtin or a manifest
//! Composition          → composition.rs (fold layers + verify locked
//!                        monotonicity D1–D4, section 7.4; governed Exceptions)
//! Conflict detection   → duplicate IDs at the same level (section 7.6)
//! Tool validation      → compilable regexes, detect builtin-only in Phase 0
//! Index generation     → event → candidate rules
//! Snapshot creation    → immutable artifact with a canonical hash
//! ```
//!
//! ── Composition ─────────────────────────────────────────────────────────
//! When a rule id appears across composition layers (section 7.2), [`compose`]
//! folds them into one effective rule and, against any `locked` ancestor,
//! verifies the composed rule is at least as restrictive (D1–D4). A weakening
//! is a compile error unless a governed `Exception` (section 7.4) authorizes it.
//! A flat workspace is one layer, so composition is an identity there.
//!
//! The compiler is a PURE FUNCTION config → snapshot: it does not import the
//! runtime (forbidden edge compiler ⇏ runtime), does not evaluate events,
//! knows nothing about sessions. That is why local and CI, running this same
//! code over the same workspace, produce bit-for-bit the same hash
//! (invariant 9).

use crate::composition::{AppliedException, ComposeLayer, ExceptionInput, Inheritance, compose};
use crate::snapshot::{
    CompiledAgent, CompiledBranch, CompiledComponent, CompiledContainment, CompiledEnforcement,
    CompiledPrecondition, CompiledRequirement, CompiledRule, CompiledScope, CompiledSkill,
    CompiledToolCall, CompiledToolRef, CompiledWhen, ExternalToolDef, OutputKind, Snapshot,
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
    /// Governed exceptions applied during composition (section 7.4). Surfaced so
    /// the CLI records each as a `human` decision in the ledger — a relaxation of
    /// a `locked` rule is audited, never silent.
    pub applied_exceptions: Vec<AppliedException>,
}

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error(
        "duplicate id `{0}`: two components at the same authority level (section 7.6) — resolution required, never silent"
    )]
    DuplicateId(String),
    #[error(
        "skill `{skill}`: content file `{path}` must end in `_keel.md` — keel skills carry that suffix so their provenance (delivered by keel) is legible wherever they are read"
    )]
    SkillNaming { skill: String, path: String },
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
    #[error("component `{component}` requires unknown reference `{reference}`")]
    UnresolvedComponent {
        component: String,
        reference: String,
    },
    #[error("component `{component}` has malformed reference `{reference}`; expected kind:id")]
    MalformedComponentReference {
        component: String,
        reference: String,
    },
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
    // there is no defined composition for tools/skills/agents/components, so a
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
            // Naming condition: a skill's content files MUST end in `_keel.md`,
            // so their provenance (delivered BY keel) is legible wherever they
            // are read. Enforced at compile so every skill that ever enters a
            // workspace follows it — it is a rule, not a convention.
            for path in [Some(&skill.spec.compact), skill.spec.full.as_ref()]
                .into_iter()
                .flatten()
            {
                if !path.ends_with("_keel.md") {
                    return Err(CompileError::SkillNaming {
                        skill: skill.metadata.id.clone(),
                        path: path.clone(),
                    });
                }
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
                    version: skill
                        .metadata
                        .version
                        .clone()
                        .unwrap_or_else(|| "unversioned".to_string()),
                    description: skill.spec.description.clone(),
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

    let declared_model_executors = chain
        .iter()
        .flat_map(|layer| layer.files.components.iter())
        .filter(|(kind, _)| kind == "model-executor")
        .map(|(_, component)| component.metadata.id.clone())
        .collect::<BTreeSet<_>>();

    // ── Agents: resolve only to governed ModelExecutor components. ──
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
            if !declared_model_executors.contains(&executor_id) {
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

    let mut components: BTreeMap<String, CompiledComponent> = BTreeMap::new();
    for layer in chain {
        for (kind, component) in &layer.files.components {
            let unique_id = format!("{kind}:{}", component.metadata.id);
            if !seen.insert(unique_id.clone()) {
                return Err(CompileError::DuplicateId(unique_id));
            }
            if let Some(path) = &component.spec.content
                && !layer.files.root.join(path).exists()
            {
                warnings.push(format!(
                    "component `{unique_id}`: content file `{path}` not found"
                ));
            }
            let requirements = component
                .spec
                .requirements
                .iter()
                .map(|requirement| {
                    let (required_kind, required_id) = requirement
                        .component
                        .split_once(':')
                        .ok_or_else(|| CompileError::MalformedComponentReference {
                            component: unique_id.clone(),
                            reference: requirement.component.clone(),
                        })?;
                    Ok(CompiledRequirement {
                        kind: required_kind.to_string(),
                        id: required_id.to_string(),
                        phases: requirement.phases.clone(),
                        required: requirement.required,
                        reason: requirement.reason.clone(),
                    })
                })
                .collect::<Result<Vec<_>, CompileError>>()?;
            components.insert(
                unique_id,
                CompiledComponent {
                    kind: kind.clone(),
                    id: component.metadata.id.clone(),
                    version: component
                        .metadata
                        .version
                        .clone()
                        .unwrap_or_else(|| "unversioned".to_string()),
                    content: component.spec.content.clone(),
                    inline: component.spec.inline.clone(),
                    requirements,
                    capabilities: component.spec.capabilities.clone(),
                    config: component.spec.config.clone(),
                },
            );
        }
    }

    for component in components.values() {
        for requirement in &component.requirements {
            let exists = match requirement.kind.as_str() {
                "skill" => skills.contains_key(&requirement.id),
                "agent" => agents.contains_key(&requirement.id),
                "tool" => tools.contains_key(&requirement.id),
                other => components.contains_key(&format!("{other}:{}", requirement.id)),
            };
            if !exists {
                return Err(CompileError::UnresolvedComponent {
                    component: format!("{}:{}", component.kind, component.id),
                    reference: format!("{}:{}", requirement.kind, requirement.id),
                });
            }
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

    // ── Governed exceptions (section 7.4): the only route to relax a locked
    //    rule. Collected across the chain; expiry is checked against the
    //    snapshot's own creation date (deterministic — no wall clock). ──
    let mut exceptions = Vec::new();
    for layer in chain {
        for exc in &layer.files.exceptions {
            exceptions.push(ExceptionInput {
                rule_id: exc
                    .spec
                    .rule
                    .strip_prefix("rule:")
                    .unwrap_or(&exc.spec.rule)
                    .to_string(),
                owner: exc.spec.owner.clone(),
                reason: exc.spec.reason.clone(),
                expiry: exc.spec.expiry.clone(),
                scope_include: exc
                    .spec
                    .scope
                    .paths
                    .as_ref()
                    .map(|p| p.include.clone())
                    .unwrap_or_default(),
            });
        }
    }
    let reference_date = created_at.get(0..10);

    // ── Composition + monotonicity verification (section 7.4) ──
    let composed = compose(&compose_layers, &exceptions, reference_date)?;
    for exc in &composed.applied_exceptions {
        warnings.push(format!(
            "governed exception applied (human decision, section 7.4): rule `{}` relaxed on {} by `{}`, owned by `{}`, expires {} — {}",
            exc.rule, exc.dimension, exc.violated_by, exc.owner, exc.expiry, exc.reason
        ));
    }
    let applied_exceptions = composed.applied_exceptions;

    // ── Containment: UNION across layers (restrictions only add, section 7.4
    //    monotonicity philosophy). None when no layer declares one. ──
    let containment = compile_containment(chain);

    // ── Index generation + Snapshot creation (canonical hash, invariant 9) ──
    let snapshot = Snapshot::build_with_components(
        composed.rules,
        tools,
        skills,
        agents,
        components,
        created_at,
    )?
    .with_containment(containment)?;
    Ok(CompileOutcome {
        snapshot,
        warnings,
        applied_exceptions,
    })
}

/// Composes every layer's `Containment` into one by UNION: deny-unlink globs
/// are unioned (sorted, deduped for a deterministic hash), booleans OR-ed.
/// Returns `None` when no layer declares containment.
fn compile_containment(chain: &[CompileLayer]) -> Option<CompiledContainment> {
    let mut deny_unlink = BTreeSet::new();
    let mut deny_write_outside = false;
    let mut deny_network = false;
    let mut any = false;
    for layer in chain {
        for doc in &layer.files.containments {
            any = true;
            deny_unlink.extend(doc.spec.deny_unlink.iter().cloned());
            deny_write_outside |= doc.spec.deny_write_outside;
            deny_network |= doc.spec.deny_network;
        }
    }
    if !any {
        return None;
    }
    Some(CompiledContainment {
        deny_unlink: deny_unlink.into_iter().collect(),
        deny_write_outside,
        deny_network,
    })
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
