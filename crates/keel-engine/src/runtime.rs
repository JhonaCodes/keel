// SPDX-License-Identifier: Apache-2.0
//! Rule Engine — the two execution modes of the keel (spec section 15.1, section 5.3).
//!
//! Evaluates an event against the snapshot and produces evaluations with the
//! DECLARED decision (what the rule asks for) and the EFFECTIVE decision
//! (what gets applied). Which one wins depends on the mode:
//!
//! | Mode      | effective_decision            | Entry point    | Purpose |
//! |-----------|-------------------------------|----------------|---------|
//! | `Passive` | `declared.min(Review)` — nothing blocks | `keel observe` | Telemetry (ADR-021): measure which constraints are alive, dead, mis-specified |
//! | `Enforce` | `declared` — block means BLOCK | `CapabilityManager` | A blocked action never exists as a process |
//!
//! Both modes write the same declared/effective pair to the ledger, so
//! telemetry never degrades: the Phase 0c comparison (violations reaching
//! review with vs. without enforcement) can be computed from either stream.
//!
//! BOUNDARY RULE: this module does NOT import `keel_dsl`. It only sees the
//! compiled snapshot (ADR-004: the runtime has no types to represent
//! authoring configuration).

use crate::snapshot::{
    CompiledBranch, CompiledPrecondition, CompiledRule, CompiledToolRef, EnvConstraint, Snapshot,
};
use crate::tools::{self, ExecContext};
use keel_core::event::Event;
use keel_core::{Decision, OriginClass, Verdict};
use std::path::Path;

/// Execution mode (see module header table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Telemetry only: every effective decision is capped at `review`.
    Passive,
    /// The keel holds: declared decisions apply as-is. Used exclusively by
    /// pre-action entry points in `CapabilityManager` — never by `keel observe`.
    Enforce,
}

/// Result of evaluating ONE rule against ONE event. The caller (CLI/testkit)
/// turns it into a `LedgerEntry` by adding id/ts/session — this way the
/// testkit can evaluate without a ledger and the ledger stays a pure sink.
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub rule_id: String,
    pub rule_version: Option<String>,
    pub verdict: Verdict,
    pub origin: OriginClass,
    pub declared_decision: Decision,
    pub effective_decision: Decision,
    pub latency_ms: u64,
    /// Tokens consumed: 0 throughout Phase 0 (deterministic tools only, section 4.4).
    pub tokens: u64,
    pub findings: Vec<tools::Finding>,
    /// Human-readable context: failed precondition, recorded invoke, report
    /// message. Feeds `keel explain`.
    pub detail: Option<String>,
    /// Skill refs the fired branch loads (section 10.4). The Session Manager decides
    /// whether to DELIVER content or just reference it (already loaded, section 6.5).
    pub load_skills: Vec<String>,
}

impl Evaluation {
    /// Materializes this evaluation as evidence. Lives here (not in `ledger`)
    /// because the ledger is a sink (⇏ runtime); the runtime side owns the
    /// conversion, the caller adds id/ts so the testkit can evaluate without
    /// a ledger.
    pub fn to_ledger_entry(
        &self,
        event: &Event,
        snapshot_hash: &str,
        id: String,
        ts: String,
    ) -> crate::ledger::LedgerEntry {
        let mut detail_parts: Vec<String> =
            self.findings.iter().map(|f| f.message.clone()).collect();
        if let Some(d) = &self.detail {
            detail_parts.push(d.clone());
        }
        crate::ledger::LedgerEntry {
            id,
            ts,
            session_id: event.session_id.clone(),
            snapshot_hash: snapshot_hash.to_string(),
            rule_id: self.rule_id.clone(),
            rule_version: self.rule_version.clone(),
            event_kind: event.kind,
            verdict: self.verdict,
            origin: self.origin,
            declared_decision: self.declared_decision,
            effective_decision: self.effective_decision,
            latency_ms: self.latency_ms,
            tokens: self.tokens,
            file: event.file.clone(),
            line: event.line,
            detail: if detail_parts.is_empty() {
                None
            } else {
                Some(detail_parts.join(" | "))
            },
        }
    }
}

/// Evaluates an event against all candidate rules in the snapshot.
///
/// Per-rule ladder (section 4.6, in cost order):
/// scope → when → detect (µs; a miss is NOT recorded: recording detector
/// misses would explode the ledger with no signal) → preconditions (ADR-022)
/// → validate (the real verdict) → enforcement branch → passive forcing.
pub fn evaluate_event(
    snapshot: &Snapshot,
    event: &Event,
    workspace_root: &Path,
    mode: Mode,
) -> Vec<Evaluation> {
    let ctx = ExecContext { workspace_root };
    let mut out = Vec::new();
    for &idx in snapshot.candidates(event.kind) {
        let rule = &snapshot.rules[idx];
        if let Some(eval) = evaluate_rule(snapshot, rule, event, ctx, mode) {
            out.push(eval);
        }
    }
    out
}

fn evaluate_rule(
    snapshot: &Snapshot,
    rule: &CompiledRule,
    event: &Event,
    ctx: ExecContext<'_>,
    mode: Mode,
) -> Option<Evaluation> {
    // 1. Scope: the rule does not apply outside its coverage (D1 of section 7.4).
    if let Some(scope) = &rule.scope
        && !scope.matches(event.file.as_deref(), event.language.as_deref())
    {
        return None;
    }

    // 2. When: additional activation condition (cognitive activation).
    if let Some(when) = &rule.when
        && !when.matches(&event.files)
    {
        return None;
    }

    // 3. Detect: economical prefilter. A miss = the rule does not fire = no
    //    entry (the detector never decides NOR generates evidence, section 4.5).
    if let Some(detect) = &rule.detect
        && !tools::run_detector(detect, event)
    {
        return None;
    }

    let started = std::time::Instant::now();

    // 4. Preconditions (ADR-022): state of the world, in order, fail-closed.
    //    The first one that fails yields certainty of violation: there is no
    //    point validating the content of an action whose context is already
    //    forbidden.
    for pre in &rule.preconditions {
        if !tools::run_precondition(pre, event, &snapshot.tools, ctx) {
            let declared = pre.on_fail_declared;
            return Some(finish(
                rule,
                Verdict::Invalid,
                OriginClass::Deterministic,
                declared,
                started.elapsed().as_millis() as u64,
                vec![],
                Some(precondition_detail(pre)),
                mode,
            ));
        }
    }

    // 4.5 Environment constraints (section 11.4): a denied or non-allowed
    // environment is a deterministic Invalid, evaluated BEFORE content — there
    // is no point validating an action whose environment is already forbidden.
    // It flows through the rule's `invalid` enforcement branch below, so it
    // carries the same decision, packet and skills as any other invalid finding.
    let constraint_finding = rule
        .constraints
        .as_ref()
        .and_then(|c| c.environment.as_ref())
        .and_then(|env| env_violation(env, event));

    // 5. Validate: the real verdict (0 tokens if the tool is deterministic),
    // unless a constraint already denied the environment.
    let (verdict, origin, latency_ms, findings) = if let Some(reason) = constraint_finding {
        (
            Verdict::Invalid,
            OriginClass::Deterministic,
            started.elapsed().as_millis() as u64,
            vec![tools::Finding {
                message: reason,
                file: None,
                line: None,
            }],
        )
    } else {
        match &rule.validate {
            Some(call) => {
                let outcome = tools::run_validate(call, event, &snapshot.tools, ctx);
                (
                    outcome.verdict,
                    outcome.origin,
                    started.elapsed().as_millis() as u64 + outcome.latency_ms,
                    outcome.findings,
                )
            }
            None => {
                if rule.enforcement.always.is_some() {
                    // Cognitive activation: does not govern an action; records
                    // the context load it would have delivered.
                    (
                        Verdict::Valid,
                        OriginClass::Deterministic,
                        started.elapsed().as_millis() as u64,
                        vec![],
                    )
                } else {
                    // No validate and no always: undecidable by construction.
                    (
                        Verdict::Unknown,
                        OriginClass::Deterministic,
                        started.elapsed().as_millis() as u64,
                        vec![],
                    )
                }
            }
        }
    };

    // 6. Enforcement branch according to the verdict.
    let branch = pick_branch(rule, verdict);
    let declared = branch
        .map(|b| b.decision)
        .unwrap_or_else(|| default_decision(verdict));
    let detail = branch.and_then(|b| branch_detail(b, verdict));
    let load_skills = branch.map(|b| b.load_skills.clone()).unwrap_or_default();

    let mut eval = finish(
        rule, verdict, origin, declared, latency_ms, findings, detail, mode,
    );
    eval.load_skills = load_skills;
    Some(eval)
}

/// Actionable detail for a failed precondition. `skill.loaded` names the skill
/// and how to satisfy it, so the block packet tells the model exactly what to
/// do (load it through keel) instead of a bare "precondition failed".
fn precondition_detail(pre: &CompiledPrecondition) -> String {
    if let CompiledToolRef::Builtin(id) = &pre.using
        && id == "skill.loaded"
        && let Some(skill) = pre
            .with
            .as_ref()
            .and_then(|w| w.get("id"))
            .and_then(|v| v.as_str())
    {
        return format!(
            "this action requires the keel skill `{skill}` — load it with keel.skills.load and follow it, then retry"
        );
    }
    format!("precondition failed: {}", pre.using)
}

/// Environment constraint check (section 11.4). The environment is classified from
/// the event's connection context — its `command` and `content` (e.g. a
/// connection string) — by case-insensitive WHOLE-WORD match (deterministic, 0
/// tokens): `production` does not match `reproduction`, and `local` does not
/// match `localstack`. `deny` blocks ALWAYS; a non-empty `allow` is a strict
/// allowlist: the action must name an allowed environment, otherwise it is
/// denied. Returns the reason when the constraint is violated.
fn env_violation(env: &EnvConstraint, event: &Event) -> Option<String> {
    let mut hay = String::new();
    if let Some(c) = &event.command {
        hay.push_str(&c.to_lowercase());
        hay.push(' ');
    }
    if let Some(c) = &event.content {
        hay.push_str(&c.to_lowercase());
    }
    for d in &env.deny {
        if contains_word(&hay, &d.to_lowercase()) {
            return Some(format!("denied environment `{d}` (section 11.4)"));
        }
    }
    if !env.allow.is_empty()
        && !env
            .allow
            .iter()
            .any(|a| contains_word(&hay, &a.to_lowercase()))
    {
        return Some(format!(
            "no allowed environment present (allow: {:?}) (section 11.4)",
            env.allow
        ));
    }
    None
}

/// Whole-word containment: `needle` occurs in `hay` bounded by non-alphanumeric
/// characters (or the string edges). Connection strings separate the
/// environment token with `@ / : - . _`, all non-alphanumeric, so this matches
/// `@production-db` but not `reproduction` and not `localstack`.
fn contains_word(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = hay.as_bytes();
    let mut from = 0;
    while let Some(pos) = hay[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

fn pick_branch(rule: &CompiledRule, verdict: Verdict) -> Option<&CompiledBranch> {
    let e = &rule.enforcement;
    if e.always.is_some() && rule.validate.is_none() {
        return e.always.as_ref();
    }
    match verdict {
        Verdict::Valid => e.valid.as_ref().or(e.always.as_ref()),
        Verdict::Invalid => e.invalid.as_ref(),
        Verdict::Unknown => e.unknown.as_ref(),
    }
}

/// Default decision when the author did not declare the branch. Conservative
/// without being dramatic: the compiler already normalized the section 4.7 floor on
/// the declared branches; a missing branch falls back to `review`
/// (invalid/unknown) or `allow`.
fn default_decision(verdict: Verdict) -> Decision {
    match verdict {
        Verdict::Valid => Decision::Allow,
        Verdict::Invalid | Verdict::Unknown => Decision::Review,
    }
}

/// Human-readable branch detail: which context it would have loaded, which
/// agent it would have invoked (RECORDED, not executed — Phase 2), which
/// message it reports.
fn branch_detail(branch: &CompiledBranch, verdict: Verdict) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(msg) = &branch.report_message {
        parts.push(msg.clone());
    }
    if !branch.load_skills.is_empty() {
        parts.push(format!("load: {}", branch.load_skills.join(", ")));
    }
    if !branch.load_capabilities.is_empty() {
        // Phase 0 honesty (like `invoke`): capabilities are a forward-declared
        // field of the DSL (spec section 11.3, future section 9 context-economy). They are
        // recorded in the evidence so the declaration is visible, but the
        // runtime does not yet ACTIVATE/limit them — that is Phase 2. Surfacing
        // them here keeps the field honest instead of a silent no-op.
        parts.push(format!(
            "capabilities declared (activation is Phase 2): {}",
            branch.load_capabilities.join(", ")
        ));
    }
    if let Some(agent) = &branch.invoke_agent {
        // Phase 0 honesty: the invoke exists in the rule and stays in the
        // evidence, but no model is executed in the passive slice.
        parts.push(format!(
            "invoke recorded (NOT executed, Phase 2): {agent} [verdict={}]",
            serde_json::to_string(&verdict).unwrap_or_default()
        ));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" | "))
    }
}

#[allow(clippy::too_many_arguments)]
fn finish(
    rule: &CompiledRule,
    verdict: Verdict,
    origin: OriginClass,
    declared: Decision,
    latency_ms: u64,
    findings: Vec<tools::Finding>,
    detail: Option<String>,
    mode: Mode,
) -> Evaluation {
    Evaluation {
        rule_id: rule.id.clone(),
        rule_version: rule.version.clone(),
        verdict,
        origin,
        declared_decision: declared,
        // The mode decides (see module header). Passive: capped at `review`
        // (ADR-021 telemetry). Enforce: declared applies — the keel holds.
        // `min` uses the single lattice in keel-core — the same ordering law
        // the compiler's D3 verification will use in Phase 1.
        effective_decision: match mode {
            Mode::Passive => declared.min(Decision::Review),
            Mode::Enforce => declared,
        },
        latency_ms,
        tokens: 0,
        findings,
        detail,
        load_skills: Vec::new(),
    }
}

#[cfg(test)]
#[path = "../tests-unit/runtime.rs"]
mod tests;
