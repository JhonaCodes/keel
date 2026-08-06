// SPDX-License-Identifier: Apache-2.0
//! Composition and `locked` monotonicity (spec section 7.3/7.4) — the
//! `Composition` step of the section 10.1 pipeline.
//!
//! When the same rule id appears across composition layers (section 7.2,
//! highest authority first), this module folds them into ONE effective rule
//! `R'` and, if any layer declared the rule `locked`, verifies that `R'` is **at
//! least as restrictive** as the locked ancestor `R` across all four dimensions
//! (section 7.4). Weakening any dimension is a compile error carrying the exact
//! diff and the layer that introduced it — a silent weakening cannot exist.
//!
//! Composition modes (section 7.3), applied per lower layer:
//! - `merge: append` — the layer's requirements are ADDED (a join in the
//!   restriction lattice); the result can only get stricter.
//! - `overridable` — a higher authority granted replacement; the lower layer
//!   may replace the rule, exempt from the monotonicity requirement.
//! - otherwise, with a `locked` ancestor — the lower layer's definition must be
//!   at least as restrictive as the ancestor (verified D1–D4), then replaces it.
//! - otherwise (no `locked` ancestor) — the lower layer replaces freely.
//!
//! The comparator reasons over the COMPILED rule (`snapshot::CompiledRule`), so
//! decisions are already floor-normalized (section 4.7) before D3 runs.
//!
//! Scope and known, SOUND-BUT-CONSERVATIVE approximations (never allow a
//! weakening; may reject a legitimate strengthening — the escape is a governed
//! `Exception`, section 7.4):
//! - D1 compares `languages`/`include`/`exclude` by exact set membership, not by
//!   glob subsumption over resolved pattern sets. Include/languages must be a
//!   superset (or "all"); any new exclude is rejected.
//! - D2 requires the detect/validate tool call to be IDENTICAL (a lower layer
//!   may only ADD one the base lacked). The DSL has a single `validate` slot, so
//!   AND-composition of MULTIPLE validators is not yet expressible; it is
//!   deferred until `validate` is list-valued.
//! - The `locked` guarantee covers D1–D4 (scope/detect+validate/decision/load).
//!   `preconditions` and `constraints.environment` are NOT part of the
//!   monotonicity check (section 7.4 does not enumerate them); a verified-locked
//!   replacement may change them.

use crate::snapshot::{
    CompiledBranch, CompiledConstraints, CompiledEnforcement, CompiledRule, CompiledScope,
    CompiledToolCall, EnvConstraint,
};
use keel_core::Decision;
use keel_dsl::Merge;

/// The dimension along which a `locked` rule can be weakened (section 7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Coverage,
    Sensitivity,
    Consequence,
    Load,
}

impl std::fmt::Display for Dimension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Dimension::Coverage => "D1-scope",
            Dimension::Sensitivity => "D2-sensitivity",
            Dimension::Consequence => "D3-decision",
            Dimension::Load => "D4-load",
        };
        f.write_str(s)
    }
}

/// A `locked` rule was weakened by a lower layer (spec section 7.4). Carries the
/// exact diff and the layer that introduced it, mirroring the spec's error
/// object (`status: monotonicity-violation`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "monotonicity-violation: rule `{rule}` locked at `{locked_at}` was weakened by `{violated_by}` on {dimension} — {detail}"
)]
pub struct MonotonicityViolation {
    pub rule: String,
    pub locked_at: String,
    pub violated_by: String,
    pub dimension: Dimension,
    pub detail: String,
}

/// A rule's composition inheritance flags (spec section 7.3).
#[derive(Debug, Clone, Copy, Default)]
pub struct Inheritance {
    pub locked: bool,
    pub overridable: bool,
    pub merge: Option<Merge>,
}

/// One composition layer's compiled rules, tagged with the layer label used in
/// violation reports (e.g. `global`, `organization:nui`, `project:con-app`).
pub struct ComposeLayer {
    pub label: String,
    pub rules: Vec<(Inheritance, CompiledRule)>,
}

/// Folds the layers (highest authority first) into the effective rule set,
/// verifying `locked` monotonicity (section 7.4). Output is sorted by rule id
/// (invariant 9: the snapshot hash must not depend on layer/file order).
pub fn compose(chain: &[ComposeLayer]) -> Result<Vec<CompiledRule>, MonotonicityViolation> {
    use std::collections::BTreeMap;

    // rule id -> the layer stack for that id, in composition order.
    let mut by_id: BTreeMap<&str, Vec<(&str, &Inheritance, &CompiledRule)>> = BTreeMap::new();
    for layer in chain {
        for (inh, rule) in &layer.rules {
            by_id
                .entry(rule.id.as_str())
                .or_default()
                .push((layer.label.as_str(), inh, rule));
        }
    }

    let mut out = Vec::with_capacity(by_id.len());
    for (_id, stack) in by_id {
        out.push(fold_rule(&stack)?);
    }
    Ok(out)
}

/// Folds one rule id's layer stack (highest authority first) into `R'`.
fn fold_rule(
    stack: &[(&str, &Inheritance, &CompiledRule)],
) -> Result<CompiledRule, MonotonicityViolation> {
    let (first_label, first_inh, first_rule) = stack[0];
    let mut effective = first_rule.clone();
    // The layer whose definition is currently effective (provenance).
    let mut effective_label = first_label;
    // The locked ancestor `R` and where it was declared, if any.
    let mut locked_base: Option<CompiledRule> = first_inh.locked.then(|| first_rule.clone());
    let mut locked_at: Option<String> = first_inh.locked.then(|| first_label.to_string());
    let mut overridable = first_inh.overridable;

    for &(label, inh, rule) in &stack[1..] {
        if inh.merge == Some(Merge::Append) {
            // Add-only composition (section 7.3): a join that can only strengthen.
            effective =
                append(&effective, rule).map_err(|(dimension, detail)| MonotonicityViolation {
                    rule: effective.id.clone(),
                    locked_at: locked_at.clone().unwrap_or_else(|| label.to_string()),
                    violated_by: label.to_string(),
                    dimension,
                    detail,
                })?;
            // If the merging layer itself locks, the merged result is the new,
            // stricter floor for lower layers (section 7.4).
            if inh.locked {
                locked_base = Some(effective.clone());
                locked_at = Some(label.to_string());
                overridable = inh.overridable;
            }
        } else if locked_at.is_none() {
            // No locked ancestor: a lower layer replaces freely.
            effective = rule.clone();
            effective_label = label;
            locked_base = inh.locked.then(|| rule.clone());
            locked_at = inh.locked.then(|| label.to_string());
            overridable = inh.overridable;
        } else if overridable {
            // A higher authority granted replacement (section 7.4 exemption).
            effective = rule.clone();
            effective_label = label;
            locked_base = inh.locked.then(|| rule.clone());
            locked_at = inh.locked.then(|| label.to_string());
            overridable = inh.overridable;
        } else {
            // Locked, not overridable: the lower definition must be at least as
            // restrictive as the ancestor (section 7.4).
            let base = locked_base.as_ref().expect("locked_at implies locked_base");
            if let Err((dimension, detail)) = at_least_as_restrictive(rule, base) {
                return Err(MonotonicityViolation {
                    rule: rule.id.clone(),
                    locked_at: locked_at.clone().expect("locked_at set"),
                    violated_by: label.to_string(),
                    dimension,
                    detail,
                });
            }
            effective = rule.clone();
            effective_label = label;
            // A lower layer may ALSO lock: its (verified ≥) definition becomes
            // the new, stricter floor, so a further-down layer is checked
            // against IT, not only the original ancestor (section 7.4: every
            // locked ancestor is enforced, not just the highest).
            if inh.locked {
                locked_base = Some(rule.clone());
                locked_at = Some(label.to_string());
                overridable = inh.overridable;
            }
        }
    }
    // Stamp composition provenance (section 7.4) onto the effective rule.
    effective.origin_layer = Some(effective_label.to_string());
    effective.locked_at = locked_at;
    Ok(effective)
}

/// Verifies `candidate` is at least as restrictive as `base` across D1–D4
/// (spec section 7.4). Returns the first weakened dimension and a diff detail.
pub fn at_least_as_restrictive(
    candidate: &CompiledRule,
    base: &CompiledRule,
) -> Result<(), (Dimension, String)> {
    check_coverage(&candidate.scope, &base.scope)?; // D1
    check_sensitivity(candidate, base)?; // D2
    check_consequence(&candidate.enforcement, &base.enforcement)?; // D3
    check_load(&candidate.enforcement, &base.enforcement)?; // D4
    Ok(())
}

// ── D1: coverage ────────────────────────────────────────────────────────────

/// `scope(candidate) ⊇ scope(base)` (section 7.4-D1). A `None` scope is
/// universal (governs everything). Coverage shrinks — a violation — when the
/// candidate narrows `languages`/`include` or adds an `exclude`.
fn check_coverage(
    candidate: &Option<CompiledScope>,
    base: &Option<CompiledScope>,
) -> Result<(), (Dimension, String)> {
    let universal = CompiledScope::default();
    let cand = candidate.as_ref().unwrap_or(&universal);
    let base = base.as_ref().unwrap_or(&universal);

    // languages / include: an empty list means "all". A base of "all" cannot be
    // narrowed to a specific set; a specific base must be covered by candidate
    // (candidate "all", or a superset).
    covers_all_or_superset(&cand.languages, &base.languages, "languages")?;
    covers_all_or_superset(&cand.include, &base.include, "include")?;

    // exclude carves coverage OUT: the candidate must not exclude anything the
    // base did not (conservative; exact glob-intersection is deferred).
    for pat in &cand.exclude {
        if !base.exclude.contains(pat) {
            return Err((
                Dimension::Coverage,
                format!("exclude added: `{pat}` narrows the locked coverage"),
            ));
        }
    }
    Ok(())
}

/// For a coverage list where empty means "all": base "all" ⇒ candidate must be
/// "all"; base specific ⇒ candidate "all" or a superset.
fn covers_all_or_superset(
    cand: &[String],
    base: &[String],
    field: &str,
) -> Result<(), (Dimension, String)> {
    if base.is_empty() {
        // Base governs everything on this axis.
        if cand.is_empty() {
            return Ok(());
        }
        return Err((
            Dimension::Coverage,
            format!("{field} narrowed: locked rule governs all, candidate restricts to {cand:?}"),
        ));
    }
    if cand.is_empty() {
        return Ok(()); // candidate governs all → superset of any specific base
    }
    for item in base {
        if !cand.contains(item) {
            return Err((
                Dimension::Coverage,
                format!(
                    "{field} narrowed: `{item}` is governed by the locked rule but not the candidate"
                ),
            ));
        }
    }
    Ok(())
}

// ── D2: sensitivity ──────────────────────────────────────────────────────────

/// The detect/validate chain of a `locked` rule is not substitutable from below
/// (section 7.4-D2): a lower layer may ADD a detector/validator the base lacks,
/// never replace one the base referenced.
fn check_sensitivity(
    candidate: &CompiledRule,
    base: &CompiledRule,
) -> Result<(), (Dimension, String)> {
    not_substituted(&candidate.detect, &base.detect, "detect")?;
    not_substituted(&candidate.validate, &base.validate, "validate")?;
    Ok(())
}

fn not_substituted(
    cand: &Option<CompiledToolCall>,
    base: &Option<CompiledToolCall>,
    field: &str,
) -> Result<(), (Dimension, String)> {
    let Some(base_call) = base else {
        return Ok(()); // base referenced nothing here → candidate may add one
    };
    match cand {
        None => Err((
            Dimension::Sensitivity,
            format!("{field} removed: the locked rule's {field} is gone in the candidate"),
        )),
        Some(cand_call) if tool_calls_equal(cand_call, base_call) => Ok(()),
        Some(_) => Err((
            Dimension::Sensitivity,
            format!("{field} substituted: the locked rule's {field} tool/params were replaced"),
        )),
    }
}

fn tool_calls_equal(a: &CompiledToolCall, b: &CompiledToolCall) -> bool {
    a.using == b.using && a.with == b.with
}

// ── D3: consequence ───────────────────────────────────────────────────────────

/// `decision(candidate) ≥ decision(base)` per enforcement branch (section
/// 7.4-D3), over the floor-normalized decisions.
fn check_consequence(
    candidate: &CompiledEnforcement,
    base: &CompiledEnforcement,
) -> Result<(), (Dimension, String)> {
    for (name, cand, base) in [
        ("invalid", &candidate.invalid, &base.invalid),
        ("unknown", &candidate.unknown, &base.unknown),
        ("valid", &candidate.valid, &base.valid),
    ] {
        let Some(base_branch) = base else { continue };
        match cand {
            None => {
                return Err((
                    Dimension::Consequence,
                    format!("decision dropped: `{name}` branch of the locked rule is gone"),
                ));
            }
            Some(cand_branch) if cand_branch.decision >= base_branch.decision => {}
            Some(cand_branch) => {
                return Err((
                    Dimension::Consequence,
                    format!(
                        "decision downgraded on `{name}`: {} < locked {}",
                        decision_str(cand_branch.decision),
                        decision_str(base_branch.decision)
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn decision_str(d: Decision) -> &'static str {
    match d {
        Decision::Allow => "allow",
        Decision::Review => "review",
        Decision::Block => "block",
        Decision::DenyPendingApproval => "deny-pending-approval",
    }
}

// ── D4: cognitive load ────────────────────────────────────────────────────────

/// The skills/capabilities a `locked` rule loads are neither removable nor
/// substitutable, only extensible (section 7.4-D4).
fn check_load(
    candidate: &CompiledEnforcement,
    base: &CompiledEnforcement,
) -> Result<(), (Dimension, String)> {
    for (name, cand, base) in [
        ("invalid", &candidate.invalid, &base.invalid),
        ("unknown", &candidate.unknown, &base.unknown),
        ("valid", &candidate.valid, &base.valid),
        ("always", &candidate.always, &base.always),
    ] {
        let Some(base_branch) = base else { continue };
        if base_branch.load_skills.is_empty() && base_branch.load_capabilities.is_empty() {
            continue;
        }
        let empty = CompiledBranch {
            decision: Decision::Allow,
            load_skills: vec![],
            load_capabilities: vec![],
            report_message: None,
            invoke_agent: None,
        };
        let cand_branch = cand.as_ref().unwrap_or(&empty);
        if let Some(missing) = first_missing(&cand_branch.load_skills, &base_branch.load_skills) {
            return Err((
                Dimension::Load,
                format!("load skill dropped on `{name}`: `{missing}` is no longer loaded"),
            ));
        }
        if let Some(missing) = first_missing(
            &cand_branch.load_capabilities,
            &base_branch.load_capabilities,
        ) {
            return Err((
                Dimension::Load,
                format!("load capability dropped on `{name}`: `{missing}` is no longer loaded"),
            ));
        }
    }
    Ok(())
}

/// The first element of `base` absent from `cand`, if any.
fn first_missing(cand: &[String], base: &[String]) -> Option<String> {
    base.iter().find(|b| !cand.contains(b)).cloned()
}

// ── merge: append (the restriction-lattice join, section 7.3) ─────────────────

/// Joins `add` onto `base`: the result covers at least as much, decides at least
/// as strictly, and loads at least as much. Sensitivity is add-only: `add` may
/// supply a detector/validator `base` lacks, but may not substitute one it has
/// (that would be a D2 weakening even under `append`).
fn append(base: &CompiledRule, add: &CompiledRule) -> Result<CompiledRule, (Dimension, String)> {
    let mut out = base.clone();

    // Scope: union coverage (empty = all dominates); intersect excludes.
    out.scope = Some(join_scope(&base.scope, &add.scope));

    // Sensitivity: add-only, never substitute.
    out.detect = join_tool(&base.detect, &add.detect, "detect")?;
    out.validate = join_tool(&base.validate, &add.validate, "validate")?;

    // Preconditions: append (dedup exact repeats).
    for pre in &add.preconditions {
        if !out.preconditions.iter().any(|p| {
            p.using == pre.using && p.with == pre.with && p.on_fail_declared == pre.on_fail_declared
        }) {
            out.preconditions.push(pre.clone());
        }
    }

    // Consequence + load: escalate decision, union loads, per branch.
    out.enforcement = join_enforcement(&base.enforcement, &add.enforcement);

    // Constraints: union denies, intersect allows (both stricter).
    out.constraints = join_constraints(&base.constraints, &add.constraints);

    Ok(out)
}

fn join_scope(base: &Option<CompiledScope>, add: &Option<CompiledScope>) -> CompiledScope {
    let universal = CompiledScope::default();
    let b = base.as_ref().unwrap_or(&universal);
    let a = add.as_ref().unwrap_or(&universal);
    CompiledScope {
        languages: union_all_dominates(&b.languages, &a.languages),
        include: union_all_dominates(&b.include, &a.include),
        exclude: intersect(&b.exclude, &a.exclude),
    }
}

/// Union where an empty list means "all" and therefore dominates.
fn union_all_dominates(a: &[String], b: &[String]) -> Vec<String> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = a.to_vec();
    for x in b {
        if !out.contains(x) {
            out.push(x.clone());
        }
    }
    out.sort();
    out
}

fn intersect(a: &[String], b: &[String]) -> Vec<String> {
    let mut out: Vec<String> = a.iter().filter(|x| b.contains(x)).cloned().collect();
    out.sort();
    out.dedup();
    out
}

fn join_tool(
    base: &Option<CompiledToolCall>,
    add: &Option<CompiledToolCall>,
    field: &str,
) -> Result<Option<CompiledToolCall>, (Dimension, String)> {
    match (base, add) {
        (Some(b), Some(a)) if !tool_calls_equal(b, a) => Err((
            Dimension::Sensitivity,
            format!("merge cannot substitute `{field}`: append may only add, not replace it"),
        )),
        (Some(b), _) => Ok(Some(b.clone())),
        (None, Some(a)) => Ok(Some(a.clone())),
        (None, None) => Ok(None),
    }
}

fn join_enforcement(base: &CompiledEnforcement, add: &CompiledEnforcement) -> CompiledEnforcement {
    CompiledEnforcement {
        invalid: join_branch(&base.invalid, &add.invalid),
        unknown: join_branch(&base.unknown, &add.unknown),
        valid: join_branch(&base.valid, &add.valid),
        always: join_branch(&base.always, &add.always),
    }
}

fn join_branch(
    base: &Option<CompiledBranch>,
    add: &Option<CompiledBranch>,
) -> Option<CompiledBranch> {
    match (base, add) {
        (Some(b), Some(a)) => {
            let mut skills = b.load_skills.clone();
            for s in &a.load_skills {
                if !skills.contains(s) {
                    skills.push(s.clone());
                }
            }
            let mut caps = b.load_capabilities.clone();
            for c in &a.load_capabilities {
                if !caps.contains(c) {
                    caps.push(c.clone());
                }
            }
            Some(CompiledBranch {
                decision: b.decision.max(a.decision),
                load_skills: skills,
                load_capabilities: caps,
                report_message: b
                    .report_message
                    .clone()
                    .or_else(|| a.report_message.clone()),
                invoke_agent: b.invoke_agent.clone().or_else(|| a.invoke_agent.clone()),
            })
        }
        (Some(b), None) => Some(b.clone()),
        (None, Some(a)) => Some(a.clone()),
        (None, None) => None,
    }
}

fn join_constraints(
    base: &Option<CompiledConstraints>,
    add: &Option<CompiledConstraints>,
) -> Option<CompiledConstraints> {
    match (base, add) {
        (Some(b), Some(a)) => Some(CompiledConstraints {
            environment: join_env(&b.environment, &a.environment),
        }),
        (Some(b), None) => Some(b.clone()),
        (None, Some(a)) => Some(a.clone()),
        (None, None) => None,
    }
}

fn join_env(base: &Option<EnvConstraint>, add: &Option<EnvConstraint>) -> Option<EnvConstraint> {
    match (base, add) {
        (Some(b), Some(a)) => {
            let mut deny = b.deny.clone();
            for d in &a.deny {
                if !deny.contains(d) {
                    deny.push(d.clone());
                }
            }
            deny.sort();
            // Allow is a strict allowlist: the stricter combined allowlist is the
            // intersection (both must permit it). An empty allow means "no
            // allowlist restriction", so an empty side does not constrain.
            let allow = match (b.allow.is_empty(), a.allow.is_empty()) {
                (true, true) => Vec::new(),
                (false, true) => b.allow.clone(),
                (true, false) => a.allow.clone(),
                (false, false) => intersect(&b.allow, &a.allow),
            };
            Some(EnvConstraint { allow, deny })
        }
        (Some(b), None) => Some(b.clone()),
        (None, Some(a)) => Some(a.clone()),
        (None, None) => None,
    }
}

#[cfg(test)]
#[path = "../tests-unit/composition.rs"]
mod tests;
