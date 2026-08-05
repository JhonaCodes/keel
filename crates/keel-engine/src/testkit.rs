// SPDX-License-Identifier: Apache-2.0
//! Testkit — runs the workspace's `kind: RuleTest` documents.
//!
//! It is the FUNCTIONAL EQUIVALENCE criterion of Phase 0a (spec §15.1: every
//! already-deployed gate, verified case by case against the behavior of the
//! original) and the GATE of `keel compile` (spec §10.2: a snapshot is
//! published only if compilation AND its tests pass).
//!
//! BOUNDARY NOTE: this module imports `keel_dsl` (RuleTests are authoring)
//! AND `runtime` (it executes them). That is legal: the testkit is
//! compiler-side orchestration. What does NOT happen is the reverse —
//! `compile` does not call the testkit (the CLI orchestrates compile → test
//! → publish) and the runtime knows nothing about these types.

use crate::runtime::{evaluate_event, Evaluation, Mode};
use crate::snapshot::Snapshot;
use keel_dsl::RuleTestDoc;
use std::path::Path;

#[derive(Debug)]
pub struct TestReport {
    pub test_id: String,
    pub target_rule: String,
    pub passed: bool,
    pub detail: String,
}

/// Runs all RuleTests against a snapshot (in staging: without a ledger —
/// test evaluations are not session evidence).
pub fn run_tests(snapshot: &Snapshot, tests: &[RuleTestDoc], workspace_root: &Path) -> Vec<TestReport> {
    tests.iter().map(|t| run_one(snapshot, t, workspace_root)).collect()
}

fn run_one(snapshot: &Snapshot, test: &RuleTestDoc, workspace_root: &Path) -> TestReport {
    let target = test
        .spec
        .target
        .strip_prefix("rule:")
        .unwrap_or(&test.spec.target)
        .to_string();

    let evals = evaluate_event(snapshot, &test.spec.event, workspace_root, Mode::Passive);
    let hit: Option<&Evaluation> = evals.iter().find(|e| e.rule_id == target);
    let expect = &test.spec.expect;

    let mut failures = Vec::new();

    match (expect.fired, hit) {
        (Some(false), Some(_)) => {
            failures.push("expected fired=false but the rule evaluated the event".to_string());
        }
        (Some(false), None) => { /* correct: it does not fire */ }
        (_, None) => {
            failures.push(format!(
                "rule `{target}` did not evaluate the event (scope/detect not applicable?)"
            ));
        }
        (_, Some(eval)) => {
            if let Some(v) = expect.verdict {
                if eval.verdict != v {
                    failures.push(format!(
                        "verdict: expected {v:?}, got {:?}",
                        eval.verdict
                    ));
                }
            }
            if let Some(d) = expect.decision {
                // The DECLARED decision is compared: the RuleTest asserts what
                // the rule would ask for; passive forcing is orthogonal to the
                // test.
                if eval.declared_decision != d {
                    failures.push(format!(
                        "decision (declared): expected {d:?}, got {:?}",
                        eval.declared_decision
                    ));
                }
            }
            if let Some(o) = expect.origin {
                if eval.origin != o {
                    failures.push(format!("origin: expected {o:?}, got {:?}", eval.origin));
                }
            }
        }
    }

    TestReport {
        test_id: test.metadata.id.clone(),
        target_rule: target,
        passed: failures.is_empty(),
        detail: if failures.is_empty() {
            "ok".to_string()
        } else {
            failures.join("; ")
        },
    }
}