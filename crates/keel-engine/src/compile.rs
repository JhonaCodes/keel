// SPDX-License-Identifier: Apache-2.0
//! Compiler — the spec §10.1 pipeline, trimmed down to Phase 0.
//!
//! ```text
//! Parse                → workspace.rs (YAML → Document, schema-validated)
//! Schema validation    → keel-dsl::schema (ADR-023 active)
//! Reference resolution → every tool ref resolves to a builtin or a manifest
//! Composition          → *** DOCUMENTED STUB *** (see below)
//! Conflict detection   → duplicate IDs at the same level (§7.6)
//! Tool validation      → compilable regexes, detect builtin-only in Phase 0
//! Index generation     → event → candidate rules
//! Snapshot creation    → immutable artifact with a canonical hash
//! ```
//!
//! ── Composition: why it is a no-op in Phase 0 ───────────────────────────
//! The `locked` monotonicity check (spec §7.4, D1–D4) operates on the
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

use crate::snapshot::{
    CompiledBranch, CompiledEnforcement, CompiledPrecondition, CompiledRule, CompiledScope,
    CompiledSkill, CompiledToolCall, CompiledToolRef, CompiledWhen, ExternalToolDef, OutputKind,
    Snapshot,
};
use crate::tools::{BUILTIN_DETECTORS, BUILTIN_PRECONDITIONS};
use crate::workspace::WorkspaceFiles;
use keel_core::{Decision, Reversibility};
use keel_dsl::{Branch, Enforcement, OnFail, RuleDoc, ToolDoc, ToolRef};
use std::collections::BTreeMap;

const DEFAULT_TOOL_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug)]
pub struct CompileOutcome {
    pub snapshot: Snapshot,
    /// Non-blocking debt (§6.5: exemplar/report missing on block rules;
    /// §4.7 floors applied by normalization). The rule ledger starts here:
    /// debt is declared, not swallowed.
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("duplicate id `{0}`: two components at the same authority level (§7.6) — resolution required, never silent")]
    DuplicateId(String),
    #[error("rule `{rule}`: unresolvable reference `{reference}` — {hint}")]
    UnresolvedTool {
        rule: String,
        reference: String,
        hint: String,
    },
    #[error("rule `{rule}`: invalid regex in detect/validate: {source}")]
    InvalidRegex {
        rule: String,
        #[source]
        source: regex::Error,
    },
    #[error("rule `{rule}`: external detect not supported in Phase 0 (the §11.4 corpus uses builtin detectors only); use a builtin or move the tool to validate")]
    ExternalDetect { rule: String },
    #[error("rule `{rule}`: reviewAfter `{value}` is not a valid ISO-8601 duration")]
    BadReviewAfter { rule: String, value: String },
    #[error("snapshot not serializable: {0}")]
    Snapshot(#[from] serde_json::Error),
}

/// Compiles the loaded workspace into an immutable snapshot.
pub fn compile(files: &WorkspaceFiles, created_at: String) -> Result<CompileOutcome, CompileError> {
    let mut warnings = Vec::new();

    // ── Conflict detection (§7.6): the compiler does not resolve silently ──
    let mut seen = std::collections::BTreeSet::new();
    for doc in files.rules.iter().map(|r| &r.metadata.id) {
        if !seen.insert(doc.clone()) {
            return Err(CompileError::DuplicateId(doc.clone()));
        }
    }

    // ── Tool manifests ──
    let mut tools: BTreeMap<String, ExternalToolDef> = BTreeMap::new();
    for tool in &files.tools {
        if !seen.insert(tool.metadata.id.clone()) {
            return Err(CompileError::DuplicateId(tool.metadata.id.clone()));
        }
        tools.insert(tool.metadata.id.clone(), compile_tool(tool));
    }

    // ── Skill manifests (§14.12): paths validated, content read at delivery ──
    let mut skills: BTreeMap<String, CompiledSkill> = BTreeMap::new();
    for skill in &files.skills {
        if !seen.insert(skill.metadata.id.clone()) {
            return Err(CompileError::DuplicateId(skill.metadata.id.clone()));
        }
        if !files.root.join(&skill.spec.compact).exists() {
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

    // ── Composition: DOCUMENTED STUB (see module header, §7.4) ──
    composition_stub();

    // ── Reference resolution + Tool validation + Policy compilation ──
    let mut rules = Vec::with_capacity(files.rules.len());
    for rule in &files.rules {
        rules.push(compile_rule(rule, &tools, &skills, &mut warnings)?);
    }

    // ── Index generation + Snapshot creation (canonical hash, invariant 9) ──
    let snapshot = Snapshot::build(rules, tools, skills, created_at)?;
    Ok(CompileOutcome { snapshot, warnings })
}

/// `Composition` step of the §10.1 pipeline — deliberate no-op in Phase 0.
///
/// HERE is where the D1–D4 `locked` monotonicity check (§7.4) will live once
/// there is a second authority layer to compose. It is kept as a named
/// function so the pipeline declares ALL its steps even when one does not
/// operate yet: a silently omitted step is the failure mode this system
/// fights.
fn composition_stub() {}

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

    // reviewAfter validated at compile time: prune (§7.7) depends on it.
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
                    return Err(CompileError::ExternalDetect { rule: id.clone() })
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
                            hint: "in validate only text.contains/text.regex are allowed as builtins".into(),
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

    // ── Reference resolution for load.skills (§10.1): a skill a rule loads
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

    // Rule debt (§6.5/§10.4): a block with neither message nor skills
    // produces findings open to interpretation — declared as a warning.
    if let Some(inv) = &enforcement.invalid {
        if inv.decision >= Decision::Block
            && inv.report_message.is_none()
            && inv.load_skills.is_empty()
        {
            warnings.push(format!(
                "rule `{id}`: invalid branch with decision block but no report.message or load.skills — rule debt (§6.5): an ambiguous block reproduces the failure mode the system fights"
            ));
        }
    }

    Ok(CompiledRule {
        id: id.clone(),
        version: rule.metadata.version.clone(),
        author: rule.metadata.author.clone().unwrap_or_default(),
        adr_ref: rule.metadata.adr_ref.clone().unwrap_or_default(),
        review_after,
        reversibility: spec.reversibility,
        scope: spec.scope.as_ref().map(|s| CompiledScope {
            languages: s.languages.clone(),
            include: s.paths.as_ref().map(|p| p.include.clone()).unwrap_or_default(),
            exclude: s.paths.as_ref().map(|p| p.exclude.clone()).unwrap_or_default(),
        }),
        on: spec.on.clone(),
        when: spec.when.as_ref().map(|w| CompiledWhen {
            any_files_touch: w
                .any
                .iter()
                .filter_map(|c| c.files_touch.clone())
                .collect(),
            all_files_touch: w
                .all
                .iter()
                .filter_map(|c| c.files_touch.clone())
                .collect(),
        }),
        detect,
        preconditions,
        validate,
        enforcement,
        constraints: spec.constraints.clone(),
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
    regex::Regex::new(pattern).map(|_| ()).map_err(|source| CompileError::InvalidRegex {
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

    // Floor §4.7 normalization (ADR-017): on an IRREVERSIBLE rule the
    // `unknown` branch has `deny-pending-approval` as its floor, regardless
    // of what it composes to. It is normalized at COMPILE time (a static
    // property) so the runtime never has to reason about reversibility.
    if reversibility == Some(Reversibility::Irreversible) {
        let floor = Decision::DenyPendingApproval;
        match &mut unknown {
            Some(branch) if branch.decision < floor => {
                warnings.push(format!(
                    "rule `{rule_id}`: unknown branch raised to deny-pending-approval (floor §4.7 for irreversible actions: uncertainty escalates to a human, never to a model)"
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
        load_skills: b.load.as_ref().map(|l| l.skills.clone()).unwrap_or_default(),
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
mod tests {
    use super::*;
    use crate::workspace::WorkspaceFiles;
    use keel_dsl::{parse_documents, Document};

    fn files_from_yaml(yaml: &str) -> WorkspaceFiles {
        let mut files = WorkspaceFiles::empty(std::path::PathBuf::from("/tmp/ws"));
        for doc in parse_documents(yaml).unwrap() {
            match doc {
                Document::Rule(r) => files.rules.push(r),
                Document::Tool(t) => files.tools.push(t),
                Document::Skill(k) => files.skills.push(k),
                Document::Agent(a) => files.agents.push(a),
                Document::AgentExecutor(x) => files.executors.push(x),
                Document::RuleTest(t) => files.tests.push(t),
                Document::Workspace(_) => {}
            }
        }
        files
    }

    const IRREVERSIBLE_NO_UNKNOWN: &str = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: gate, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  reversibility: irreversible
  on: [command.requested]
  validate: { using: "tool:sql.classify" }
  enforcement:
    invalid: { decision: block }
    valid: { decision: allow }
---
apiVersion: keel/v1alpha1
kind: Tool
metadata: { id: sql.classify }
spec: { command: ["true"], output: exit-code }
"#;

    /// Floor §4.7: irreversible without an unknown branch → the compiler
    /// creates it with deny-pending-approval. Uncertainty never goes without
    /// a destination.
    #[test]
    fn irreversible_rule_gets_unknown_floor() {
        let files = files_from_yaml(IRREVERSIBLE_NO_UNKNOWN);
        let out = compile(&files, "2026-01-01T00:00:00Z".into()).unwrap();
        let rule = out.snapshot.rule_by_id("gate").unwrap();
        assert_eq!(
            rule.enforcement.unknown.as_ref().unwrap().decision,
            Decision::DenyPendingApproval
        );
    }

    /// Floor §4.7: an unknown branch DECLARED below the floor is RAISED with
    /// a warning (never silently).
    #[test]
    fn irreversible_unknown_below_floor_is_raised_with_warning() {
        let yaml = IRREVERSIBLE_NO_UNKNOWN.replace(
            "    invalid: { decision: block }",
            "    invalid: { decision: block }\n    unknown: { decision: review }",
        );
        let files = files_from_yaml(&yaml);
        let out = compile(&files, "2026-01-01T00:00:00Z".into()).unwrap();
        let rule = out.snapshot.rule_by_id("gate").unwrap();
        assert_eq!(
            rule.enforcement.unknown.as_ref().unwrap().decision,
            Decision::DenyPendingApproval
        );
        assert!(out.warnings.iter().any(|w| w.contains("floor §4.7")));
    }

    #[test]
    fn unresolved_tool_reference_is_a_compile_error() {
        let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r1, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  on: [file.edited]
  validate: { using: "tool:ghost.tool" }
  enforcement:
    invalid: { decision: block }
"#;
        let files = files_from_yaml(yaml);
        let err = compile(&files, "t".into()).unwrap_err();
        assert!(matches!(err, CompileError::UnresolvedTool { .. }));
    }

    #[test]
    fn duplicate_rule_ids_conflict_loudly() {
        let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: dup, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  on: [file.edited]
  validate: { using: "builtin:text.contains", with: { value: "x" } }
  enforcement: { invalid: { decision: review } }
---
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: dup, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  on: [file.edited]
  validate: { using: "builtin:text.contains", with: { value: "y" } }
  enforcement: { invalid: { decision: review } }
"#;
        let files = files_from_yaml(yaml);
        assert!(matches!(
            compile(&files, "t".into()).unwrap_err(),
            CompileError::DuplicateId(_)
        ));
    }

    #[test]
    fn invalid_regex_fails_at_compile_not_runtime() {
        let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r1, author: t, adrRef: "adr:ADR-1", reviewAfter: P6M }
spec:
  on: [file.edited]
  detect: { using: "builtin:text.regex", with: { pattern: "([unclosed" } }
  validate: { using: "builtin:text.contains", with: { value: "x" } }
  enforcement: { invalid: { decision: review } }
"#;
        let files = files_from_yaml(yaml);
        assert!(matches!(
            compile(&files, "t".into()).unwrap_err(),
            CompileError::InvalidRegex { .. }
        ));
    }

    /// Defense in depth: the schema catches the grossly invalid ("6 months"
    /// does not even start with P); the compiler catches what LOOKS like
    /// ISO-8601 but is not ("P6X" passes the schema's `^P` pattern).
    #[test]
    fn bad_review_after_fails_compile() {
        let yaml = r#"
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r1, author: t, adrRef: "adr:ADR-1", reviewAfter: "P6X" }
spec:
  on: [file.edited]
  validate: { using: "builtin:text.contains", with: { value: "x" } }
  enforcement: { invalid: { decision: review } }
"#;
        let files = files_from_yaml(yaml);
        assert!(matches!(
            compile(&files, "t".into()).unwrap_err(),
            CompileError::BadReviewAfter { .. }
        ));
    }

    /// Mismo workspace compilado dos veces → mismo hash (invariante 9).
    #[test]
    fn same_workspace_same_hash() {
        let files = files_from_yaml(IRREVERSIBLE_NO_UNKNOWN);
        let a = compile(&files, "2026-01-01T00:00:00Z".into()).unwrap();
        let b = compile(&files, "2026-12-31T23:59:59Z".into()).unwrap();
        assert_eq!(a.snapshot.hash, b.snapshot.hash);
    }
}