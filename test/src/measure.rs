// SPDX-License-Identifier: Apache-2.0
//! Phase 0c enforcement-measurement harness (spec section 15.1).
//!
//! MEASUREMENT, NOT ENGINE CODE. This orchestrates the shipped `keel` binary
//! (`compile` / `observe` / `gate`) over a fixed dataset in BOTH passive and
//! enforce modes, then aggregates the result from the ledger with a read-only
//! SQLite connection. No evaluation logic is added to the runtime — the spec
//! is explicit that Phase 0c "is measurement, not code".
//!
//! The primary metric is the spec's original comparison: architectural
//! violations reaching human review WITH vs WITHOUT active enforcement. In the
//! passive arm every violation reaches review (nothing blocks); in the enforce
//! arm the inner-ring violations are prevented pre-action and never reach
//! review. The delta is that difference — the value enforcement adds over the
//! ledger-only baseline.

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A violation reaches the ledger as `invalid` with a blocking declared
/// decision. These are the SQL literals the ledger stores (enums serialized
/// with their JSON quotes — see keel-engine::ledger). The authority for this
/// vocabulary is keel-core's `Decision`/`Verdict`/`EventKind`; the test
/// `ledger_literals_match_engine_vocabulary` anchors these literals to it so a
/// serde-rename drift fails loudly instead of silently reading 0.
const BLOCKING: &str = r#"('"block"','"deny-pending-approval"')"#;
/// Inner-ring events (spec section 5.3): the only ones a block can PREVENT
/// pre-action. `completion.requested` is preventable too but produces no
/// invalid entry of its own (its denial reuses the live blockers).
const INNER_RING: &str =
    r#"('"command.requested"','"transition.requested"','"delivery.requested"')"#;

// ------------------------------------------------------------------ dataset

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub task_count: usize,
}

#[derive(Debug, Deserialize)]
pub struct Expected {
    #[serde(default)]
    pub id: String,
    pub tasks: Vec<ExpectedTask>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedTask {
    pub id: String,
    #[serde(default)]
    pub gate_exits: Vec<i32>,
    #[serde(default)]
    pub expected_valid: u64,
    #[serde(default)]
    pub expected_violations: u64,
    #[serde(default)]
    pub expected_unknown: u64,
    #[serde(default)]
    pub expected_prevented: u64,
}

pub struct TaskFile {
    pub id: String,
    pub path: PathBuf,
    /// Raw JSONL event lines, in order (blank lines dropped).
    pub events: Vec<String>,
}

pub struct Dataset {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub expected: Expected,
    pub tasks: Vec<TaskFile>,
}

impl Dataset {
    /// Loads a dataset directory: `manifest.yaml`, `expected.yaml`, a
    /// `workspace/` (a complete keel workspace) and `tasks/*.jsonl`.
    pub fn load(root: &Path) -> Result<Dataset> {
        let manifest: Manifest = read_yaml(&root.join("manifest.yaml"))?;
        let expected: Expected = read_yaml(&root.join("expected.yaml"))?;

        let tasks_dir = root.join("tasks");
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&tasks_dir)
            .with_context(|| format!("no tasks/ dir at {}", tasks_dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        paths.sort();

        let mut tasks = Vec::new();
        for path in paths {
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("could not read task {}", path.display()))?;
            let events: Vec<String> = raw
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            tasks.push(TaskFile { id, path, events });
        }
        if tasks.is_empty() {
            bail!("dataset {} has no tasks", root.display());
        }
        Ok(Dataset {
            root: root.to_path_buf(),
            manifest,
            expected,
            tasks,
        })
    }

    fn workspace(&self) -> PathBuf {
        self.root.join("workspace")
    }

    fn event_count(&self) -> usize {
        self.tasks.iter().map(|t| t.events.len()).sum()
    }
}

// ------------------------------------------------------------------ options

pub struct Options {
    /// Path to the compiled `keel` binary. Defaults to the workspace target.
    pub keel_bin: PathBuf,
    /// A delta rate at or above which the project continues (spec's "material,
    /// sustained delta"). Below it, the viable project is the ledger-only subset.
    pub min_delta_rate: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            keel_bin: default_keel_bin(),
            min_delta_rate: 0.10,
        }
    }
}

/// Resolves the `keel` binary the same way the black-box tests do
/// (`test/../target/<profile>/keel`), overridable by `KEEL_BIN`.
pub fn default_keel_bin() -> PathBuf {
    if let Ok(p) = std::env::var("KEEL_BIN") {
        return PathBuf::from(p);
    }
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // keel/
    p.push("target");
    p.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    p.push("keel");
    p
}

// ------------------------------------------------------------------ report

#[derive(Debug, Serialize)]
pub struct Report {
    pub dataset_id: String,
    pub snapshot_hash: String,
    pub task_count: usize,
    pub event_count: usize,
    pub primary: Primary,
    pub secondary: Secondary,
    pub verdict: String,
}

#[derive(Debug, Serialize)]
pub struct Primary {
    pub violations: u64,
    pub reach_review_passive: u64,
    pub reach_review_enforce: u64,
    pub delta: u64,
    pub delta_rate: f64,
    pub prevented: u64,
    pub exit2_count: u64,
}

#[derive(Debug, Serialize)]
pub struct Secondary {
    /// MEASURED: count of `unknown` verdicts (the undecidable tail).
    pub unknown_count: u64,
    /// GAP: tokens are structurally 0 in Phase 0 (deterministic tools); real
    /// token accounting is inv 13 / the executor path, not this experiment.
    pub tokens_total: u64,
    /// From dataset labels (expected.yaml): actual violations minus expected.
    /// Positive = over-firing (false positives); negative = under-firing.
    pub false_positive_estimate: i64,
    /// PROXY: repeated finding at the same rule+file+line within a session.
    pub oscillations: Vec<OscillationRow>,
    /// PROXY: mean tool latency per rule (not end-to-end per-edit latency).
    pub latency_by_rule: Vec<LatencyRow>,
}

#[derive(Debug, Serialize)]
pub struct OscillationRow {
    pub rule_id: String,
    pub session_id: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub count: u64,
}

#[derive(Debug, Serialize)]
pub struct LatencyRow {
    pub rule_id: String,
    pub avg_latency_ms: f64,
    pub evaluations: u64,
}

// ------------------------------------------------------------------ run

/// Runs the full experiment and returns the report. `out_dir` receives the two
/// ephemeral workspaces (with their own ledgers) so the arms never contaminate
/// each other. Nothing is written to the dataset directory.
pub fn run(dataset: &Dataset, out_dir: &Path, opts: &Options) -> Result<Report> {
    if !opts.keel_bin.exists() {
        bail!(
            "keel binary not found at {} — build it (cargo build) or set KEEL_BIN",
            opts.keel_bin.display()
        );
    }
    std::fs::create_dir_all(out_dir)?;

    // Two isolated workspaces, each a copy of the dataset's — this fixes "the
    // same underlying rules" across both arms (spec 15.1) and keeps their
    // ledgers apart.
    let passive_ws = out_dir.join("passive-ws");
    let enforce_ws = out_dir.join("enforce-ws");
    // Fresh state every run: the ledger is append-only, so a stale `.keel-state`
    // from a previous run would double-count. Start each arm from empty.
    let _ = std::fs::remove_dir_all(&passive_ws);
    let _ = std::fs::remove_dir_all(&enforce_ws);
    copy_dir(&dataset.workspace(), &passive_ws)?;
    copy_dir(&dataset.workspace(), &enforce_ws)?;
    compile(&opts.keel_bin, &passive_ws)?;
    compile(&opts.keel_bin, &enforce_ws)?;
    let snapshot_hash = doctor_snapshot_hash(&opts.keel_bin, &enforce_ws)?;

    // Passive arm: observe replays each task; every effective decision is
    // capped at `review` — nothing blocks, everything is recorded.
    for task in &dataset.tasks {
        observe(&opts.keel_bin, &passive_ws, &task.path)?;
    }

    // Enforce arm: gate evaluates one event at a time (its stdin contract) in
    // Enforce mode; a blocked preventable action exits 2.
    let mut exit2_count = 0u64;
    for task in &dataset.tasks {
        for event in &task.events {
            let code = gate(&opts.keel_bin, &enforce_ws, &task.id, event)?;
            if code == 2 {
                exit2_count += 1;
            }
        }
    }

    // Aggregate from the two ledgers, read-only.
    let passive = Ledger::open_ro(&ledger_path(&passive_ws))?;
    let enforce = Ledger::open_ro(&ledger_path(&enforce_ws))?;

    // The cross-arm arithmetic below is only coherent because both arms replay
    // the SAME events against the SAME rules, so they DECLARE the same
    // violations (only `effective` differs). Make that premise explicit — a
    // mismatch means the two arms diverged and the delta would be meaningless.
    let violations = passive.violations()?;
    let enforce_violations = enforce.violations()?;
    if violations != enforce_violations {
        bail!(
            "cross-arm mismatch: passive declared {violations} violations, enforce declared \
             {enforce_violations} — both arms must see the same declared violations"
        );
    }
    let prevented = enforce.prevented()?;
    let reach_review_passive = violations;
    let reach_review_enforce = violations.saturating_sub(prevented);
    let delta = reach_review_passive.saturating_sub(reach_review_enforce);
    let delta_rate = if violations == 0 {
        0.0
    } else {
        delta as f64 / violations as f64
    };

    // Cross-check: every prevented violation must correspond to an exit-2 gate
    // invocation (prevented ⊆ exit-2). exit-2 also includes completion denials,
    // so it is a superset, not an identity.
    if prevented > exit2_count {
        bail!("inconsistent measurement: prevented={prevented} exceeds exit-2 count={exit2_count}");
    }

    let expected_violations: u64 = dataset
        .expected
        .tasks
        .iter()
        .map(|t| t.expected_violations)
        .sum();
    let false_positive_estimate = enforce_violations as i64 - expected_violations as i64;

    let secondary = Secondary {
        unknown_count: enforce.unknown_count()?,
        tokens_total: enforce.tokens_total()?,
        false_positive_estimate,
        oscillations: enforce.oscillations(3)?,
        latency_by_rule: enforce.latency_by_rule()?,
    };

    let verdict = decide(violations, delta_rate, opts.min_delta_rate);

    Ok(Report {
        dataset_id: dataset.manifest.id.clone(),
        snapshot_hash,
        task_count: dataset.tasks.len(),
        event_count: dataset.event_count(),
        primary: Primary {
            violations,
            reach_review_passive,
            reach_review_enforce,
            delta,
            delta_rate,
            prevented,
            exit2_count,
        },
        secondary,
        verdict,
    })
}

/// The continuation criterion (spec 15.1). A material, sustained delta →
/// CONTINUE. No violations at all → INCONCLUSIVE (the dataset cannot decide).
/// A measurable-but-immaterial delta → SMALLER-PROJECT: the viable project is
/// the ledger + passive-evaluation subset (the spec's ordering note).
fn decide(violations: u64, delta_rate: f64, threshold: f64) -> String {
    if violations == 0 {
        "INCONCLUSIVE".into()
    } else if delta_rate >= threshold {
        "CONTINUE".into()
    } else {
        "SMALLER-PROJECT".into()
    }
}

// ------------------------------------------------------------------ ledger (read-only)

struct Ledger {
    conn: Connection,
}

impl Ledger {
    fn open_ro(path: &Path) -> Result<Ledger> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("could not open ledger {}", path.display()))?;
        Ok(Ledger { conn })
    }

    fn count(&self, where_clause: &str) -> Result<u64> {
        let sql = format!("SELECT COUNT(*) FROM evidence WHERE {where_clause}");
        Ok(self.conn.query_row(&sql, [], |r| r.get(0))?)
    }

    fn violations(&self) -> Result<u64> {
        self.count(&format!(
            r#"verdict = '"invalid"' AND declared_decision IN {BLOCKING}"#
        ))
    }

    fn prevented(&self) -> Result<u64> {
        self.count(&format!(
            r#"verdict = '"invalid"' AND declared_decision IN {BLOCKING}
               AND effective_decision IN {BLOCKING} AND event_kind IN {INNER_RING}"#
        ))
    }

    fn unknown_count(&self) -> Result<u64> {
        self.count(r#"verdict = '"unknown"'"#)
    }

    fn tokens_total(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COALESCE(SUM(tokens),0) FROM evidence", [], |r| {
                r.get(0)
            })?)
    }

    fn oscillations(&self, threshold: u64) -> Result<Vec<OscillationRow>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT rule_id, session_id, file, line, COUNT(*) AS n
               FROM evidence
               WHERE verdict = '"invalid"'
               GROUP BY rule_id, session_id, file, line
               HAVING COUNT(*) >= ?1
               ORDER BY n DESC"#,
        )?;
        let rows = stmt.query_map([threshold], |row| {
            Ok(OscillationRow {
                rule_id: row.get(0)?,
                session_id: row.get(1)?,
                file: row.get(2)?,
                line: row.get(3)?,
                count: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn latency_by_rule(&self) -> Result<Vec<LatencyRow>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT rule_id, AVG(latency_ms), COUNT(*)
               FROM evidence GROUP BY rule_id ORDER BY rule_id"#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(LatencyRow {
                rule_id: row.get(0)?,
                avg_latency_ms: row.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                evaluations: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

// ------------------------------------------------------------------ keel driver

fn ledger_path(ws: &Path) -> PathBuf {
    ws.join(".keel-state").join("ledger.sqlite")
}

fn compile(bin: &Path, ws: &Path) -> Result<()> {
    let (code, _out, err) = run_keel(bin, &["compile", "--workspace", path(ws)], None)?;
    if code != 0 {
        bail!("keel compile failed for {}: {err}", ws.display());
    }
    Ok(())
}

fn observe(bin: &Path, ws: &Path, events: &Path) -> Result<()> {
    let (code, _out, err) = run_keel(
        bin,
        &["observe", "--workspace", path(ws), "--events", path(events)],
        None,
    )?;
    if code != 0 {
        bail!("keel observe failed for {}: {err}", events.display());
    }
    Ok(())
}

/// Runs `keel gate` for a single event on stdin; returns its exit code.
fn gate(bin: &Path, ws: &Path, session: &str, event: &str) -> Result<i32> {
    let (code, _out, _err) = run_keel(
        bin,
        &["gate", "--workspace", path(ws), "--session", session],
        Some(event),
    )?;
    Ok(code)
}

/// Reads the published snapshot hash from `keel doctor` (proves "the same
/// rules" in the report without reaching into the snapshot format).
fn doctor_snapshot_hash(bin: &Path, ws: &Path) -> Result<String> {
    let (_code, out, _err) = run_keel(bin, &["doctor", "--workspace", path(ws)], None)?;
    for line in out.lines() {
        if let Some(idx) = line.find("sha256:") {
            // Capture the `sha256:` prefix plus its hex body, stopping at the
            // trailing space / `(3 rules)`.
            let hash: String = line[idx..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == ':')
                .collect();
            return Ok(hash);
        }
    }
    Ok("sha256:unknown".into())
}

fn run_keel(bin: &Path, args: &[&str], stdin: Option<&str>) -> Result<(i32, String, String)> {
    use std::io::Write;
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn {}", bin.display()))?;
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .context("no stdin")?
            .write_all(input.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn path(p: &Path) -> &str {
    p.to_str().expect("dataset paths are utf-8")
}

/// Recursively copies a directory tree (dataset workspace → ephemeral copy).
fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn read_yaml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    serde_yaml_ng::from_str(&raw).with_context(|| format!("invalid YAML in {}", path.display()))
}

// ------------------------------------------------------------------ rendering

impl Report {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Writes both report artifacts (`report.json` + `report.md`) into `out_dir`.
    /// Producing these is the "done" of the harness (PROGRAMA_DE_TRABAJO.md:48).
    pub fn write_to(&self, out_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(out_dir)?;
        std::fs::write(out_dir.join("report.json"), self.to_json()?)?;
        std::fs::write(out_dir.join("report.md"), self.to_markdown())?;
        Ok(())
    }

    /// The human decision document (spec 15.1). Every secondary metric is
    /// annotated MEASURED / PROXY / GAP — honesty about what the ledger can and
    /// cannot supply is itself a spec requirement.
    pub fn to_markdown(&self) -> String {
        let p = &self.primary;
        let s = &self.secondary;
        let mut m = String::new();
        m.push_str(&format!("# Phase 0c report — {}\n\n", self.dataset_id));
        m.push_str(&format!(
            "- snapshot: `{}`\n- tasks: {} · events: {}\n- verdict: **{}**\n\n",
            self.snapshot_hash, self.task_count, self.event_count, self.verdict
        ));

        m.push_str("## Primary metric — violations reaching human review\n\n");
        m.push_str("| | value |\n|---|---|\n");
        m.push_str(&format!(
            "| violations (invalid, blocking) | {} |\n",
            p.violations
        ));
        m.push_str(&format!(
            "| reach review — passive | {} |\n",
            p.reach_review_passive
        ));
        m.push_str(&format!(
            "| reach review — enforce | {} |\n",
            p.reach_review_enforce
        ));
        m.push_str(&format!(
            "| prevented (inner-ring blocks) | {} |\n",
            p.prevented
        ));
        m.push_str(&format!(
            "| **delta** | **{}** ({:.1}%) |\n",
            p.delta,
            p.delta_rate * 100.0
        ));
        m.push_str(&format!(
            "| exit-2 gate invocations | {} (delta ⊆ exit-2) |\n\n",
            p.exit2_count
        ));

        m.push_str("## Secondary metrics\n\n");
        m.push_str("| metric | value | quality |\n|---|---|---|\n");
        m.push_str(&format!(
            "| unknown tail | {} | MEASURED |\n",
            s.unknown_count
        ));
        m.push_str(&format!(
            "| unknown-tail tokens | {} | GAP (tokens=0 until inv 13) |\n",
            s.tokens_total
        ));
        m.push_str(&format!(
            "| false positives (vs labels) | {} | dataset-labelled |\n",
            s.false_positive_estimate
        ));
        m.push_str(&format!(
            "| oscillations (>=3) | {} | PROXY |\n",
            s.oscillations.len()
        ));
        m.push_str("| per-edit latency | see per-rule below | PROXY (tool time, not e2e) |\n\n");

        if !s.latency_by_rule.is_empty() {
            m.push_str("### Latency per rule (PROXY)\n\n");
            m.push_str("| rule | avg ms | evals |\n|---|---|---|\n");
            for r in &s.latency_by_rule {
                m.push_str(&format!(
                    "| {} | {:.1} | {} |\n",
                    r.rule_id, r.avg_latency_ms, r.evaluations
                ));
            }
            m.push('\n');
        }

        m.push_str("## Honest baseline (manual)\n\n");
        m.push_str(
            "The spec's honest baseline — the current full configuration \
             (instructions + skills + project linters) plus, where one exists, the \
             per-language alternative (a native analyzer plugin for the same rules) — \
             is NOT derivable from the ledger. Record it here by hand before treating \
             the delta above as decisive; if Keel's delta over that alternative is not \
             material, the right project is smaller than this specification.\n\n",
        );

        m.push_str("## Note on this dataset\n\n");
        m.push_str(
            "A synthetic v0 dataset proves the harness end-to-end and is reproducible; \
             the real Phase 0c decision needs recorded real agent sessions in a real \
             repository with the same model and client. See the dataset README and spec \
             section 15.1.\n",
        );
        m
    }
}
