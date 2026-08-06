// SPDX-License-Identifier: Apache-2.0
//! Subcommand implementations. Orchestration and presentation; the
//! logic lives in keel-engine.

use anyhow::{Context, Result, bail};
use keel_core::event::Event;
use keel_core::{Decision, Verdict};
use keel_engine::ledger::{Ledger, LedgerEntry};
use keel_engine::runtime::{Evaluation, Mode, evaluate_event};
use keel_engine::snapshot::Snapshot;
use keel_engine::{compile as compiler, sarif, testkit, workspace};
use std::io::BufRead;
use std::path::Path;
use std::process::ExitCode;

pub(crate) fn now_ts() -> String {
    jiff::Timestamp::now().to_string()
}

pub(crate) fn new_ev_id() -> String {
    format!("ev_{}", ulid::Ulid::new().to_string().to_lowercase())
}

// ─────────────────────────── workspace init ───────────────────────────

/// Generates ONLY the structure: the rules are written by the user. Keel
/// ships no default rules — it does not know what constraints your project
/// has, and pretending it does would be exactly the generic prose it fights.
/// The only things generated are COMMENTED-OUT TEMPLATES (they do not compile
/// until you make them your own) that document the anatomy of the DSL.
pub fn workspace_init(path: &Path) -> Result<ExitCode> {
    if path.join("workspace.yaml").exists() {
        bail!("`{}` is already an Keel workspace", path.display());
    }
    for dir in ["rules", "tools", "tests", "fixtures"] {
        std::fs::create_dir_all(path.join(dir))?;
    }

    std::fs::write(
        path.join("workspace.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Workspace
metadata: { id: my-workspace }
spec:
  description: Keel workspace (Phase 0 — passive evaluation and telemetry)
"#,
    )?;

    // Commented-out template — .example extension: the loader does NOT load it.
    // The user copies it to rules/<name>.yaml and makes it their own.
    std::fs::write(
        path.join("rules/rule.yaml.example"),
        r#"# Keel rule template (rename to <name>.yaml to activate it).
#
# Anatomy (spec section 11): the rule DECLARES; the tool IMPLEMENTS (it is code).
# author, adrRef and reviewAfter are MANDATORY (ADR-023): every rule has
# an owner, a decision that justifies it and a review date — so that
# `keel prune` can propose deleting it BACKED BY DATA once it dies.
#
# apiVersion: keel/v1alpha1
# kind: Rule
# metadata:
#   id: my-project.my-rule          # unique within the workspace
#   version: 1.0.0
#   author: your-name
#   adrRef: adr:ADR-001             # the design decision that originates it
#   reviewAfter: P6M                # ISO-8601 review window
# spec:
#   reversibility: reversible       # irreversible => unknown escalates to a human (section 4.7)
#   scope:
#     languages: [python]           # optional
#     paths: { include: ["src/**"], exclude: ["src/legacy/**"] }
#   on: [file.edited]               # events: file.edited, command.requested, ...
#   detect:                         # cheap prefilter — NEVER decides (section 4.5)
#     using: builtin:text.contains  # or builtin:text.regex / builtin:command.classify
#     with: { value: "pattern" }
#   validate:                       # the real verdict: valid|invalid|unknown
#     using: tool:my-analyzer       # declare the tool in tools/ (or a text builtin)
#   enforcement:
#     invalid:
#       decision: block             # in Phase 0 it is recorded; nothing blocks yet
#       report: { message: "what to fix and how" }
#     unknown: { decision: review }
#     valid: { decision: allow }
"#,
    )?;

    std::fs::write(
        path.join("tools/tool.yaml.example"),
        r#"# External tool template (rename to <name>.yaml to activate it).
# The tool is CODE (spec section 4.4): it receives the event as JSON via stdin and
# responds per `output`. Relative paths resolve against this workspace.
#
# apiVersion: keel/v1alpha1
# kind: Tool
# metadata: { id: my-analyzer, version: 0.1.0 }
# spec:
#   command: [python3, bin/my_analyzer.py]
#   timeoutMs: 5000
#   output: verdict-json   # {"verdict":"valid|invalid|unknown","findings":[...]}
#                          # also: sarif | exit-code (0=valid,1=invalid,other=unknown)
"#,
    )?;

    std::fs::write(
        path.join("tests/test.yaml.example"),
        r#"# RuleTest template (rename to <name>.yaml to activate it).
# Every rule deserves its cases: `keel compile` does NOT publish a snapshot
# if a RuleTest fails (section 10.2). It is your safety net for editing rules
# without fear.
#
# apiVersion: keel/v1alpha1
# kind: RuleTest
# metadata: { id: my-rule.basic-case }
# spec:
#   target: rule:my-project.my-rule
#   event:
#     kind: file.edited
#     file: src/example.py
#     content: "code that violates the rule"
#   expect: { verdict: invalid, decision: block, origin: deterministic }
"#,
    )?;

    // Runtime state kept out of version control (spec section 8.4 / invariant 5).
    std::fs::write(path.join(".gitignore"), ".keel-state/\n")?;

    println!("workspace created at {}", path.display());
    println!("structure: rules/ tools/ tests/ fixtures/ (.example templates included)");
    println!("1. write your first rule in rules/ (see rules/rule.yaml.example)");
    println!("2. keel compile   # publishes the snapshot");
    println!("3. keel observe   # evaluates events passively and feeds the ledger");
    Ok(ExitCode::SUCCESS)
}

// ─────────────────────────── compile ───────────────────────────

/// ATOMIC compilation (spec section 10.2): staging → RuleTests → publish only if
/// they pass, retaining the last valid snapshot for rollback (invariant 7).
///
/// Composition-aware (spec section 7): the workspace is loaded as its layers
/// (section 8.5). If it carries a binding (`.keel/project.yaml`), the chain is
/// resolved by repository identity (section 7.1) and composed with `locked`
/// monotonicity verified (section 7.4). A flat workspace is a single layer, so
/// the same path compiles it unchanged.
pub fn compile(root: &Path) -> Result<ExitCode> {
    use keel_engine::lock::ProjectBinding;
    use keel_engine::workspace::{Layer, load_layered};

    let layered = load_layered(root)?;
    let binding = ProjectBinding::load(root).ok();

    // Select the composition chain.
    let selected: Vec<&Layer> = match &binding {
        Some(b) => {
            let chain = keel_engine::resolution::resolve(&layered, b, None)?;
            if !chain.matched_project {
                eprintln!(
                    "warning: bound project `{}` matched no project layer — it contributes no rules",
                    b.project
                );
            }
            if let keel_engine::resolution::IdentityStatus::Advisory(msg) = &chain.identity {
                eprintln!("warning: repository identity advisory (section 13.3): {msg}");
            }
            chain
                .layer_indices
                .iter()
                .map(|&i| &layered.layers[i])
                .collect()
        }
        None => {
            if layered.layers.len() == 1 {
                layered.layers.iter().collect()
            } else {
                bail!(
                    "layered workspace has {} layers but no binding — run `keel bind` to select the project chain",
                    layered.layers.len()
                );
            }
        }
    };

    let chain: Vec<compiler::CompileLayer> = selected
        .iter()
        .map(|l| compiler::CompileLayer {
            label: layer_label(l),
            files: &l.files,
        })
        .collect();
    let outcome = compiler::compile_layered(&chain, now_ts())?;

    // Gate: the configuration tests (aggregated across the chain) decide the
    // publication.
    let tests: Vec<_> = selected
        .iter()
        .flat_map(|l| l.files.tests.iter().cloned())
        .collect();
    let reports = testkit::run_tests(&outcome.snapshot, &tests, root);
    let failed: Vec<_> = reports.iter().filter(|r| !r.passed).collect();

    if !failed.is_empty() {
        eprintln!(
            "staging compilation OK, but {} RuleTest(s) fail:",
            failed.len()
        );
        for r in &failed {
            eprintln!("  FAIL {} → {}", r.test_id, r.detail);
        }
        eprintln!("snapshot NOT published; the last-known-good is retained (section 10.2)");
        return Ok(ExitCode::FAILURE);
    }

    let paths = workspace::WorkspaceFiles::empty(root.to_path_buf());
    let state = paths.state_dir();
    std::fs::create_dir_all(&state)?;
    let snap_path = paths.snapshot_path();
    if snap_path.exists() {
        // Invariant 7: retain the last valid snapshot.
        std::fs::rename(&snap_path, paths.snapshot_prev_path())?;
    }
    outcome.snapshot.save(&snap_path)?;

    println!("snapshot published  {}", outcome.snapshot.hash);
    println!("rules               {}", outcome.snapshot.rules.len());
    if outcome.snapshot.rules.is_empty() {
        println!(
            "  (workspace has no rules yet — write the first one in rules/, template at rules/rule.yaml.example)"
        );
    }
    println!("external tools      {}", outcome.snapshot.tools.len());
    println!("rule tests          {} (all green)", reports.len());
    for w in &outcome.warnings {
        println!("warning: {w}");
    }
    Ok(ExitCode::SUCCESS)
}

/// The label a layer carries in composition/monotonicity reports and in the
/// lock's `composition` list: `global`, `organization:nui`, `project:con-app`.
fn layer_label(layer: &keel_engine::workspace::Layer) -> String {
    use keel_engine::workspace::LayerId;
    let kind = match layer.id {
        LayerId::Global => "global",
        LayerId::Organization => "organization",
        LayerId::Platform => "platform",
        LayerId::Project => "project",
        LayerId::Team => "team",
        LayerId::Profile => "profile",
    };
    match &layer.name {
        Some(name) => format!("{kind}:{name}"),
        None => kind.to_string(),
    }
}

// ─────────────────────────── observe ───────────────────────────

/// Phase 0b (ADR-021): passive evaluation. Reads JSONL events, evaluates them
/// against the published snapshot, forces every effective decision to `review`
/// and appends to the ledger. NOTHING BLOCKS: the product is the telemetry.
pub fn observe(
    root: &Path,
    events_file: Option<&Path>,
    session: Option<String>,
) -> Result<ExitCode> {
    let files = workspace::load(root)?;
    let snapshot = Snapshot::load(&files.snapshot_path())
        .context("no published snapshot — run `keel compile` first")?;
    std::fs::create_dir_all(files.state_dir())?;
    let ledger = Ledger::open(&files.ledger_path())?;

    println!(
        "PASSIVE MODE (Phase 0b) — snapshot {} · {} rules",
        snapshot.hash,
        snapshot.rules.len()
    );
    println!("every effective decision is forced to `review`; nothing blocks (ADR-021)\n");

    let reader: Box<dyn BufRead> = match events_file {
        Some(p) => Box::new(std::io::BufReader::new(
            std::fs::File::open(p).with_context(|| format!("could not open {}", p.display()))?,
        )),
        None => Box::new(std::io::stdin().lock()),
    };

    let (mut n_events, mut n_evals) = (0u64, 0u64);
    let (mut n_invalid, mut n_unknown, mut n_valid) = (0u64, 0u64, 0u64);

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let mut event: Event = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(e) => {
                // A malformed event does not take down the observation: it is
                // reported and we move on (replay must be robust to dirty lines).
                eprintln!("event skipped (invalid JSON): {e}");
                continue;
            }
        };
        if event.session_id.is_none() {
            event.session_id = session.clone();
        }
        n_events += 1;

        for eval in evaluate_event(&snapshot, &event, &files.root, Mode::Passive) {
            n_evals += 1;
            match eval.verdict {
                Verdict::Invalid => n_invalid += 1,
                Verdict::Unknown => n_unknown += 1,
                Verdict::Valid => n_valid += 1,
            }
            let entry = to_ledger_entry(&eval, &event, &snapshot, new_ev_id(), now_ts());
            ledger.append(&entry)?;
            println!(
                "{}  {}  rule={} verdict={} origin={} declared={} effective={} {}ms",
                entry.id,
                json_plain(&entry.event_kind),
                entry.rule_id,
                json_plain(&entry.verdict),
                json_plain(&entry.origin),
                json_plain(&entry.declared_decision),
                json_plain(&entry.effective_decision),
                entry.latency_ms,
            );
        }
    }

    println!(
        "\n{n_events} events · {n_evals} evaluations → invalid={n_invalid} unknown={n_unknown} valid={n_valid}"
    );
    println!("telemetry: keel prune · traceability: keel explain <ev_id>");
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn to_ledger_entry(
    eval: &Evaluation,
    event: &Event,
    snapshot: &Snapshot,
    id: String,
    ts: String,
) -> LedgerEntry {
    let mut detail_parts: Vec<String> = eval.findings.iter().map(|f| f.message.clone()).collect();
    if let Some(d) = &eval.detail {
        detail_parts.push(d.clone());
    }
    LedgerEntry {
        id,
        ts,
        session_id: event.session_id.clone(),
        snapshot_hash: snapshot.hash.to_string(),
        rule_id: eval.rule_id.clone(),
        rule_version: eval.rule_version.clone(),
        event_kind: event.kind,
        verdict: eval.verdict,
        origin: eval.origin,
        declared_decision: eval.declared_decision,
        effective_decision: eval.effective_decision,
        latency_ms: eval.latency_ms,
        tokens: eval.tokens,
        file: event.file.clone(),
        line: event.line,
        detail: if detail_parts.is_empty() {
            None
        } else {
            Some(detail_parts.join(" | "))
        },
    }
}

/// Serde form without quotes for presentation ("invalid", not "\"invalid\"").
fn json_plain<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

// ─────────────────────────── explain ───────────────────────────

pub fn explain(root: &Path, ev_id: &str, as_sarif: bool) -> Result<ExitCode> {
    let files = workspace::load(root)?;
    let ledger = Ledger::open(&files.ledger_path())?;
    let Some(entry) = ledger.get(ev_id)? else {
        bail!("evidence `{ev_id}` does not exist in the ledger");
    };

    if as_sarif {
        println!(
            "{}",
            serde_json::to_string_pretty(&sarif::to_sarif(&[&entry]))?
        );
        return Ok(ExitCode::SUCCESS);
    }

    println!("evidence    {}", entry.id);
    println!("recorded    {}", entry.ts);
    if let Some(s) = &entry.session_id {
        println!("session     {s}");
    }
    println!("snapshot    {}", entry.snapshot_hash);

    // Rule provenance (ADR-023): who it belongs to and which decision justifies it.
    match Snapshot::load(&files.snapshot_path()) {
        Ok(snap) => match snap.rule_by_id(&entry.rule_id) {
            Some(rule) => println!(
                "rule        {} v{} (author: {}, {}, reviewAfter: {})",
                rule.id,
                rule.version.as_deref().unwrap_or("?"),
                rule.author,
                rule.adr_ref,
                rule.review_after
            ),
            None => println!(
                "rule        {} (no longer in the current snapshot — the evidence preserves it)",
                entry.rule_id
            ),
        },
        Err(_) => println!("rule        {}", entry.rule_id),
    }

    let location = match (&entry.file, entry.line) {
        (Some(f), Some(l)) => format!(" {f}:{l}"),
        (Some(f), None) => format!(" {f}"),
        _ => String::new(),
    };
    println!("event       {}{location}", json_plain(&entry.event_kind));
    println!(
        "verdict     {} (origin: {})",
        json_plain(&entry.verdict),
        json_plain(&entry.origin)
    );
    println!(
        "decision    declared {} → effective {} (passive mode: nothing blocks)",
        json_plain(&entry.declared_decision),
        json_plain(&entry.effective_decision)
    );
    println!(
        "cost        {}ms · {} tokens",
        entry.latency_ms, entry.tokens
    );
    if let Some(d) = &entry.detail {
        println!("detail      {d}");
    }
    Ok(ExitCode::SUCCESS)
}

// ─────────────────────────── prune ───────────────────────────

/// The lifecycle against the graveyard (spec section 7.7, ADR-023): propose with
/// data; a human decides; record with class `human`.
pub fn prune(
    root: &Path,
    record: Option<String>,
    decision: Option<String>,
    by: Option<String>,
    reason: Option<String>,
) -> Result<ExitCode> {
    let files = workspace::load(root)?;
    let ledger = Ledger::open(&files.ledger_path())?;

    // --record branch: record the human decision.
    if let Some(rule_id) = record {
        let (Some(decision), Some(by)) = (decision, by) else {
            bail!("--record requires --decision (keep|adjust|prune) and --by <who>");
        };
        if !["keep", "adjust", "prune"].contains(&decision.as_str()) {
            bail!("--decision must be keep, adjust or prune");
        }
        let id = format!("hd_{}", ulid::Ulid::new().to_string().to_lowercase());
        ledger.record_human_decision(
            &rule_id,
            &decision,
            &by,
            reason.as_deref(),
            &id,
            &now_ts(),
        )?;
        println!("human decision recorded ({id}): {rule_id} → {decision} (by {by})");
        if decision == "prune" {
            println!(
                "remember: deleting the rule means editing its file in rules/ — the system proposes, the human executes"
            );
        }
        return Ok(ExitCode::SUCCESS);
    }

    let snapshot = Snapshot::load(&files.snapshot_path())
        .context("no published snapshot — run `keel compile` first")?;
    let stats = ledger.rule_stats()?;
    let now = jiff::Timestamp::now();

    println!("keel prune — lifecycle backed by evidence (section 7.7)\n");
    for rule in &snapshot.rules {
        let stat = stats.iter().find(|s| s.rule_id == rule.id);
        println!(
            "rule: {:<40} adr: {:<12} author: {}",
            rule.id, rule.adr_ref, rule.author
        );

        let Some(stat) = stat else {
            println!("  no evaluations yet → no data (telemetry starts with `keel observe`)\n");
            continue;
        };

        let last_invalid = stat.last_invalid_ts.as_deref().unwrap_or("never");
        println!(
            "  evaluations: {:<6} invalid: {:<5} unknown: {:<5} last invalid: {}",
            stat.evaluations, stat.invalid, stat.unknown, last_invalid
        );

        // Has the review window expired? Measured from the first observed
        // evidence: without observation there is no window to run.
        let window_expired = review_window_expired(&stat.first_ts, &rule.review_after, now);

        let proposal = match (window_expired, stat.invalid, stat.unknown, stat.evaluations) {
            (false, ..) => "keep (review window still open)".to_string(),
            (true, 0, _, evals) => format!(
                "DELETION CANDIDATE (evidence: 0 invalid across {evals} evaluations over the whole window) — with data, not with guts"
            ),
            (true, _, unk, evals) if unk * 2 > evals => "adjust (high unknown tail: under-specified rule — push the semantics into structure, section 4.6)".to_string(),
            (true, ..) => "keep (active and healthy)".to_string(),
        };
        println!("  → {proposal}");
        println!(
            "    record: keel prune --record {} --decision <keep|adjust|prune> --by <who>\n",
            rule.id
        );
    }

    let decisions = ledger.human_decisions(None)?;
    if !decisions.is_empty() {
        println!("recorded human decisions (class `human`, section 6.4):");
        for d in decisions {
            println!(
                "  {} {} → {} (by {}{})",
                d.ts,
                d.rule_id,
                d.decision,
                d.decided_by,
                d.reason.map(|r| format!(": {r}")).unwrap_or_default()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn review_window_expired(first_ts: &str, review_after: &str, now: jiff::Timestamp) -> bool {
    let Ok(first) = first_ts.parse::<jiff::Timestamp>() else {
        return false;
    };
    let Ok(span) = review_after.parse::<jiff::Span>() else {
        return false;
    };
    let zoned = first.to_zoned(jiff::tz::TimeZone::UTC);
    match zoned.checked_add(span) {
        Ok(deadline) => now >= deadline.timestamp(),
        Err(_) => false,
    }
}

// ─────────────────────────── test ───────────────────────────

pub fn test(root: &Path) -> Result<ExitCode> {
    let files = workspace::load(root)?;
    let outcome = compiler::compile(&files, now_ts())?;
    let reports = testkit::run_tests(&outcome.snapshot, &files.tests, &files.root);

    if reports.is_empty() {
        println!(
            "no RuleTests in tests/ — Phase 0a requires verifying every gate case by case (section 15.1)"
        );
        return Ok(ExitCode::SUCCESS);
    }

    let mut failed = 0;
    for r in &reports {
        if r.passed {
            println!("PASS {} ({})", r.test_id, r.target_rule);
        } else {
            failed += 1;
            println!("FAIL {} ({}) → {}", r.test_id, r.target_rule, r.detail);
        }
    }
    println!("\n{} tests, {} failures", reports.len(), failed);
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

// ─────────────────────────── doctor ───────────────────────────

pub fn doctor(root: &Path) -> Result<ExitCode> {
    let mut failures = 0u32;
    let mut check = |name: &str, result: std::result::Result<String, String>| match result {
        Ok(msg) => println!("OK   {name}: {msg}"),
        Err(msg) => {
            failures += 1;
            println!("FAIL {name}: {msg}");
        }
    };

    // 1. Workspace parses (schema included).
    let files = match workspace::load(root) {
        Ok(f) => {
            check(
                "workspace",
                Ok(format!(
                    "{} rules, {} tools, {} tests",
                    f.rules.len(),
                    f.tools.len(),
                    f.tests.len()
                )),
            );
            f
        }
        Err(e) => {
            check("workspace", Err(e.to_string()));
            println!("\n1 check failed — without a workspace there is nothing else to verify");
            return Ok(ExitCode::FAILURE);
        }
    };

    // 2. Compiles to staging (without publishing: doctor is read-only).
    let staged = match compiler::compile(&files, now_ts()) {
        Ok(o) => {
            check(
                "compile (staging)",
                Ok(format!(
                    "{} · {} warnings",
                    o.snapshot.hash,
                    o.warnings.len()
                )),
            );
            Some(o.snapshot)
        }
        Err(e) => {
            check("compile (staging)", Err(e.to_string()));
            None
        }
    };

    // 3. Published snapshot (informational) and verification of its hash.
    match Snapshot::load(&files.snapshot_path()) {
        Ok(s) => check(
            "published snapshot",
            Ok(format!("{} ({} rules)", s.hash, s.rules.len())),
        ),
        Err(e) => check(
            "published snapshot",
            Err(format!("{e} — run `keel compile`")),
        ),
    }

    // 4. Ledger is accessible.
    match std::fs::create_dir_all(files.state_dir())
        .map_err(|e| e.to_string())
        .and_then(|_| Ledger::open(&files.ledger_path()).map_err(|e| e.to_string()))
        .and_then(|l| l.count().map_err(|e| e.to_string()))
    {
        Ok(n) => check("ledger", Ok(format!("{n} evidence entries"))),
        Err(e) => check("ledger", Err(e)),
    }

    // 5. External tool binaries on PATH (a missing tool is NOT fatal at
    //    runtime — it yields `unknown` — but doctor makes it visible).
    for tool in files.tools.iter() {
        let program = tool.spec.command.first().cloned().unwrap_or_default();
        let found = if program.contains('/') {
            root.join(&program).exists() || Path::new(&program).exists()
        } else {
            binary_on_path(&program)
        };
        check(
            &format!("tool `{}`", tool.metadata.id),
            if found {
                Ok(format!("`{program}` available"))
            } else {
                Err(format!(
                    "`{program}` not found (at runtime it would yield `unknown`, not a crash)"
                ))
            },
        );
    }

    // 6. Synthetic end-to-end evaluation against the staged snapshot.
    if let Some(snap) = staged {
        let synthetic = Event {
            kind: keel_core::event::EventKind::FileEdited,
            session_id: Some("doctor".into()),
            file: Some("lib/doctor_probe.dart".into()),
            language: None,
            content: Some("void main() { print('probe'); }".into()),
            line: Some(1),
            command: None,
            env: Default::default(),
            files: vec![],
        };
        let evals = evaluate_event(&snap, &synthetic, root, Mode::Passive);
        let over_review = evals
            .iter()
            .filter(|e| e.effective_decision > Decision::Review)
            .count();
        if over_review == 0 {
            check(
                "synthetic evaluation",
                Ok(format!(
                    "{} evaluations; no effective decision exceeds `review` (ADR-021)",
                    evals.len()
                )),
            );
        } else {
            check(
                "synthetic evaluation",
                Err(format!(
                    "{over_review} effective decisions exceed `review` — VIOLATES passive mode"
                )),
            );
        }
    }

    if failures == 0 {
        println!("\nall checks passed");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("\n{failures} failed check(s)");
        Ok(ExitCode::FAILURE)
    }
}

fn binary_on_path(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
}

// ─────────────────────────── bind / lock ───────────────────────────

/// `keel bind` — writes `.keel/project.yaml` (spec section 8.6). The repo holds only
/// the binding + lock, never the component definitions (invariant 4).
pub(crate) fn bind(root: &Path, project: Option<String>, org: Option<String>) -> Result<ExitCode> {
    use keel_engine::lock::ProjectBinding;

    let project = match project {
        Some(p) => p,
        None => derive_project_from_git(root).context(
            "no --project given and could not derive it from the git `origin` remote; pass --project project:org/repo",
        )?,
    };
    let workspace = org.unwrap_or_else(|| "org:local".to_string());
    let binding = ProjectBinding {
        project: project.clone(),
        workspace: workspace.clone(),
    };
    binding.write(root)?;
    println!("bound {project} → {workspace}");
    println!("wrote {}", ProjectBinding::path(root).display());
    println!("next: `keel compile` then `keel lock`.");
    Ok(ExitCode::SUCCESS)
}

/// `keel lock` — (re)generate or `--verify` the pinned resolution (spec section 8.6,
/// invariant 9). The fingerprint is the snapshot's canonical hash, so the same
/// configuration locks identically on any machine — the basis of the CI check.
pub(crate) fn lock(root: &Path, verify: bool) -> Result<ExitCode> {
    use keel_engine::lock::{Lock, ProjectBinding};

    let files = workspace::load(root)?;
    let binding = ProjectBinding::load(root)?;
    let snapshot = Snapshot::load(&files.snapshot_path())
        .context("no published snapshot — run `keel compile` first")?;
    let keel_version = env!("CARGO_PKG_VERSION");

    if verify {
        let existing = Lock::load(root)?;
        return match existing.verify(&binding, &snapshot, keel_version) {
            Ok(()) => {
                println!(
                    "lock OK — {} resolves to {}",
                    binding.project, existing.snapshot_hash
                );
                Ok(ExitCode::SUCCESS)
            }
            Err(reason) => {
                eprintln!("lock DRIFT: {reason}");
                eprintln!("review the change, then run `keel lock` to regenerate.");
                Ok(ExitCode::FAILURE)
            }
        };
    }

    let lock = Lock::generate(&binding, &snapshot, keel_version);
    lock.write(root)?;
    println!("locked {} @ {}", binding.project, lock.snapshot_hash);
    println!("wrote {}", Lock::path(root).display());
    Ok(ExitCode::SUCCESS)
}

/// Derives `project:<org>/<repo>` from the git `origin` remote. Best-effort:
/// returns `None` if git is unavailable or the remote cannot be parsed.
fn derive_project_from_git(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8(out.stdout).ok()?;
    parse_project_from_remote(url.trim())
}

/// Parses `git@host:org/repo.git` or `https://host/org/repo(.git)` into
/// `project:org/repo` (the last two path segments).
fn parse_project_from_remote(url: &str) -> Option<String> {
    let segments: Vec<&str> = url.rsplit(['/', ':']).collect();
    let repo = segments.first()?.trim_end_matches(".git");
    let org = segments.get(1)?;
    if repo.is_empty() || org.is_empty() {
        return None;
    }
    Some(format!("project:{org}/{repo}"))
}

// ─────────────────────────── ci (compliance plane) ───────────────────────────

/// Resolves the compliance plane: binding present, workspace compiles + tests
/// pass, and the lock matches a FRESH compile (no drift). Returns `true` when
/// everything resolves. This is the same engine as local, run over the pinned
/// lock — the point where `locked` becomes a guarantee (section 5.2, section 8).
fn ci_resolve_inner(root: &Path) -> Result<bool> {
    use keel_engine::lock::{Lock, ProjectBinding};

    // 1) Fail before doing work if the repo is not bound.
    let binding = match ProjectBinding::load(root) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("ci: {e}");
            return Ok(false);
        }
    };
    println!("binding    {} → {}", binding.project, binding.workspace);

    // 2) Compile from a fresh, ephemeral state and gate on RuleTests.
    let files = workspace::load(root)?;
    let outcome = compiler::compile(&files, now_ts())?;
    let reports = testkit::run_tests(&outcome.snapshot, &files.tests, &files.root);
    let failed: Vec<_> = reports.iter().filter(|r| !r.passed).collect();
    if !failed.is_empty() {
        eprintln!("ci: {} RuleTest(s) fail — not resolved", failed.len());
        for r in &failed {
            eprintln!("  FAIL {} → {}", r.test_id, r.detail);
        }
        return Ok(false);
    }
    std::fs::create_dir_all(files.state_dir())?;
    outcome.snapshot.save(&files.snapshot_path())?;
    println!(
        "compiled   {} ({} rules)",
        outcome.snapshot.hash,
        outcome.snapshot.rules.len()
    );

    // 3) The lock must exist and match the fresh compile (invariant 9).
    let lock = match Lock::load(root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ci: {e}");
            return Ok(false);
        }
    };
    match lock.verify(&binding, &outcome.snapshot, env!("CARGO_PKG_VERSION")) {
        Ok(()) => {
            println!("lock       OK ({})", lock.snapshot_hash);
            Ok(true)
        }
        Err(reason) => {
            eprintln!("ci: lock drift — {reason}");
            Ok(false)
        }
    }
}

/// `keel ci resolve` — the gate: exit non-zero if the binding/lock do not
/// resolve, before running any workflow.
pub(crate) fn ci_resolve(root: &Path) -> Result<ExitCode> {
    if ci_resolve_inner(root)? {
        println!("ci resolve OK — binding + lock resolve against a fresh compile");
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

/// `keel ci run` — resolve, then run the configured audit and point at the
/// evidence. In this minimal plane `resolve` is the gate; `run` reports where
/// the evidence (the append-only ledger) lives so CI can upload it.
pub(crate) fn ci_run(root: &Path) -> Result<ExitCode> {
    if !ci_resolve_inner(root)? {
        return Ok(ExitCode::FAILURE);
    }
    let files = workspace::load(root)?;
    println!("evidence   {}", files.ledger_path().display());
    println!("ci run OK");
    Ok(ExitCode::SUCCESS)
}

// ─────────────────────────── adapter preflight ───────────────────────────

/// `keel adapter <client> --check` — preflight the published snapshot against
/// the adapter's capability manifest (spec section 12.1, invariant 8). Rejects a
/// `block` the client cannot honor instead of assuming it. Exit 1 on any
/// unhonorable policy.
pub(crate) fn adapter_check(root: &Path, client: &str) -> Result<ExitCode> {
    use keel_engine::adapter::{AdapterManifest, preflight};

    let Some(manifest) = AdapterManifest::for_client(client) else {
        eprintln!("error: unknown adapter `{client}` (supported: claude-code)");
        return Ok(ExitCode::FAILURE);
    };
    let files = workspace::load(root)?;
    let snapshot = Snapshot::load(&files.snapshot_path())
        .context("no published snapshot — run `keel compile` first")?;

    let violations = preflight(&snapshot, &manifest);
    if violations.is_empty() {
        println!(
            "adapter {} — preflight OK ({} rules honor their blocking policy)",
            manifest.id,
            snapshot.rules.len()
        );
        return Ok(ExitCode::SUCCESS);
    }
    eprintln!(
        "adapter {} — preflight FAILED: {} unhonorable blocking policy(ies)",
        manifest.id,
        violations.len()
    );
    for v in &violations {
        eprintln!("  {} → {}", v.rule_id, v.reason);
    }
    Ok(ExitCode::FAILURE)
}
