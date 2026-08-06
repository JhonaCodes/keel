// SPDX-License-Identifier: Apache-2.0
//! End-to-end validation of the Phase 0c measurement harness (spec section 15.1)
//! AND of the shipped v0 dataset itself.
//!
//! The final test (`harness_produces_a_report_with_a_measured_delta`) is the
//! "done" gate of PROGRAMA_DE_TRABAJO.md:48 — "a reproducible dataset and a
//! report with the measured delta". It runs the real `keel` binary, so it
//! executes under `cargo test --workspace`, which builds that binary first.

use keel_core::event::Event;
use keel_tests::measure::{Dataset, Options, run};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// The shipped dataset: `keel/datasets/phase0c/v0-synthetic`.
fn v0_dataset_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // keel/
    p.push("datasets");
    p.push("phase0c");
    p.push("v0-synthetic");
    p
}

fn unique_out(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("keel-p0c-{tag}-{}-{n}", std::process::id()))
}

// ---- dataset fixture validation (Fase 1A) ----

/// A4: the manifest's declared task count matches the tasks on disk.
#[test]
fn manifest_task_count_matches_files() {
    let ds = Dataset::load(&v0_dataset_dir()).expect("dataset loads");
    assert_eq!(
        ds.manifest.task_count,
        ds.tasks.len(),
        "manifest.task_count must equal the number of task files"
    );
}

/// A2: every event line in every task deserializes into a real `keel` Event —
/// the dataset speaks the exact protocol `observe`/`gate` consume.
#[test]
fn every_task_event_is_a_valid_keel_event() {
    let ds = Dataset::load(&v0_dataset_dir()).expect("dataset loads");
    for task in &ds.tasks {
        assert!(!task.events.is_empty(), "task {} has no events", task.id);
        for (i, line) in task.events.iter().enumerate() {
            serde_json::from_str::<Event>(line)
                .unwrap_or_else(|e| panic!("task {} line {i} is not a valid Event: {e}", task.id));
        }
    }
}

/// A3: every task has a ground-truth label in expected.yaml, and every label
/// points at a real task (no orphans in either direction).
#[test]
fn every_task_is_labelled() {
    let ds = Dataset::load(&v0_dataset_dir()).expect("dataset loads");
    for task in &ds.tasks {
        assert!(
            ds.expected.tasks.iter().any(|e| e.id == task.id),
            "task {} has no label in expected.yaml",
            task.id
        );
    }
    for label in &ds.expected.tasks {
        assert!(
            ds.tasks.iter().any(|t| t.id == label.id),
            "expected.yaml labels unknown task {}",
            label.id
        );
    }
}

/// The harness hardcodes ledger vocabulary as JSON-quoted SQL literals
/// (`'"invalid"'`, `'"block"'`, `'"command.requested"'`, …). Anchor those to
/// the engine's real enum serialization: if a serde rename ever changes, this
/// fails loudly instead of letting the aggregation silently read 0.
#[test]
fn ledger_literals_match_engine_vocabulary() {
    use keel_core::event::EventKind;
    use keel_core::{Decision, Verdict};
    // Verdicts.
    assert_eq!(
        serde_json::to_string(&Verdict::Invalid).unwrap(),
        "\"invalid\""
    );
    assert_eq!(
        serde_json::to_string(&Verdict::Unknown).unwrap(),
        "\"unknown\""
    );
    // Blocking decisions the primary metric filters on.
    assert_eq!(
        serde_json::to_string(&Decision::Block).unwrap(),
        "\"block\""
    );
    assert_eq!(
        serde_json::to_string(&Decision::DenyPendingApproval).unwrap(),
        "\"deny-pending-approval\""
    );
    assert_eq!(
        serde_json::to_string(&Decision::Review).unwrap(),
        "\"review\""
    );
    // Inner-ring event kinds (the only preventable ones, spec section 5.3).
    assert_eq!(
        serde_json::to_string(&EventKind::CommandRequested).unwrap(),
        "\"command.requested\""
    );
    assert_eq!(
        serde_json::to_string(&EventKind::TransitionRequested).unwrap(),
        "\"transition.requested\""
    );
    assert_eq!(
        serde_json::to_string(&EventKind::DeliveryRequested).unwrap(),
        "\"delivery.requested\""
    );
}

// ---- harness end-to-end (Fase 1B/C/D/E — the done-when gate) ----

/// PROGRAMA:48 done-when: a reproducible dataset and a report with the measured
/// delta. Asserts the primary metric is coherent and the report is written.
#[test]
fn harness_produces_a_report_with_a_measured_delta() {
    let ds = Dataset::load(&v0_dataset_dir()).expect("dataset loads");
    let out = unique_out("e2e");
    let report = run(&ds, &out, &Options::default()).expect("harness runs end-to-end");
    report.write_to(&out).expect("report artifacts are written");

    // Primary metric, verified against the hand-computed v0 ground truth.
    assert_eq!(report.primary.violations, 5, "v0 has 5 blocking violations");
    assert_eq!(
        report.primary.prevented, 1,
        "exactly the inner-ring psql/DROP violation is prevented"
    );
    assert_eq!(report.primary.delta, 1, "delta = prevented for v0");
    assert!(
        report.primary.reach_review_passive > report.primary.reach_review_enforce,
        "enforcement must reduce violations reaching review"
    );
    assert!(
        report.primary.prevented <= report.primary.exit2_count,
        "every prevented violation must have an exit-2 gate invocation"
    );
    assert_eq!(report.verdict, "CONTINUE", "a material delta continues");

    // Secondary metrics.
    assert_eq!(
        report.secondary.unknown_count, 1,
        "one undecidable tail event"
    );
    assert_eq!(
        report.secondary.false_positive_estimate, 0,
        "measured violations match the labels — no false positives"
    );
    assert_eq!(
        report.secondary.tokens_total, 0,
        "tokens are structurally 0 in Phase 0 (deterministic tools)"
    );
    assert!(
        !report.secondary.oscillations.is_empty(),
        "the oscillation task must surface"
    );

    // The report artifacts exist and carry the load-bearing facts.
    let md = std::fs::read_to_string(out.join("report.md")).expect("report.md written");
    assert!(md.contains("delta"), "report names the delta");
    assert!(md.contains("CONTINUE"), "report states the verdict");
    assert!(md.contains("sha256:"), "report pins the snapshot hash");
    assert!(
        out.join("report.json").exists(),
        "report.json is written for machine consumption"
    );

    let _ = std::fs::remove_dir_all(&out);
}

/// D3: a degenerate dataset with no violations cannot decide the project — the
/// harness must NOT report CONTINUE (it reports INCONCLUSIVE), never a spurious
/// positive.
#[test]
fn degenerate_clean_dataset_is_inconclusive() {
    let dir = unique_out("degenerate");
    build_all_clean_dataset(&dir);

    let ds = Dataset::load(&dir).expect("degenerate dataset loads");
    let out = unique_out("degenerate-out");
    let report = run(&ds, &out, &Options::default()).expect("harness runs");

    assert_eq!(
        report.primary.violations, 0,
        "the clean dataset has no violations"
    );
    assert_eq!(report.primary.delta, 0, "no violations, no delta");
    assert_ne!(report.verdict, "CONTINUE", "no delta must not continue");
    assert_eq!(report.verdict, "INCONCLUSIVE");

    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Builds a minimal dataset that reuses the v0 rules but feeds only a clean
/// command — so nothing resolves `invalid`.
fn build_all_clean_dataset(dir: &Path) {
    use std::fs;
    let src_ws = {
        let mut p = v0_dataset_dir();
        p.push("workspace");
        p
    };
    // Copy the workspace (same rules) so the dataset is self-contained.
    copy_dir(&src_ws, &dir.join("workspace"));
    fs::create_dir_all(dir.join("tasks")).unwrap();
    fs::write(
        dir.join("manifest.yaml"),
        "id: degenerate-clean\ndescription: all-clean\nkind: synthetic\ntask_count: 1\n",
    )
    .unwrap();
    fs::write(
        dir.join("expected.yaml"),
        "id: degenerate-clean\ntasks:\n  - id: task-clean\n    gate_exits: [0]\n    expected_valid: 1\n    expected_violations: 0\n    expected_unknown: 0\n    expected_prevented: 0\n",
    )
    .unwrap();
    fs::write(
        dir.join("tasks").join("task-clean.jsonl"),
        "{\"kind\":\"command.requested\",\"command\":\"psql -c stmt\",\"content\":\"SELECT 1\",\"session_id\":\"clean\"}\n",
    )
    .unwrap();
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}
