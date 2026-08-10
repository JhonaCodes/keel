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
use keel_runtime::KnowledgeChain;
// The `keel init` scaffold content (READMEs + templates). Module namespace, so
// it coexists with the `init` function below.
use crate::init;
use std::io::BufRead;
use std::path::Path;
use std::process::ExitCode;

// Evidence identity helpers live in keel-engine (the host broker needs them
// too); the CLI re-exports them for its own modules.
pub(crate) use keel_engine::ledger::{new_ev_id, now_ts};

// ─────────────────────────── init ───────────────────────────

/// `keel init [path]` — scaffolds a workspace with its composition layers
/// (spec section 8.5) and a binding, so `keel compile` works immediately.
///
/// Keel ships NO default rules — it does not know your project's constraints,
/// and pretending it does would be the generic prose it fights. What init
/// creates is the full LAYER STRUCTURE plus COMMENTED-OUT `.example` templates
/// (the loader ignores `.example`, so nothing fires until you rename + fill it):
///
/// ```text
/// <path>/
/// ├── workspace.yaml
/// ├── .gitignore                       # ignores .keel-state/
/// ├── .keel/project.yaml               # binding → project:local/app (so compile resolves)
/// ├── global/                          # applies to EVERY project
/// │   ├── rules/rule.yaml.example      #   a `locked` global rule template
/// │   └── exceptions/exception.yaml.example
/// └── projects/app/                    # applies to THIS project only
///     ├── rules/rule.yaml.example
///     ├── tools/tool.yaml.example
///     └── tests/test.yaml.example
/// ```
pub fn init(path: &Path) -> Result<ExitCode> {
    use keel_engine::lock::ProjectBinding;

    if path.join("workspace.yaml").exists() {
        bail!("`{}` is already a Keel workspace", path.display());
    }

    // The whole spec section 8.5 tree, each directory with a README (what it is
    // + how to author it — context for a human OR an LLM filling it) and a base
    // `.example` template (the loader ignores `.example`, so nothing fires until
    // you rename + edit it). Honest about what is ACTIVE now vs org-scale /
    // engine-generated / deferred, so nothing is mistaken for working.
    let files: &[(&str, &str)] = &[
        ("README.md", init::WORKSPACE_README),
        ("workspace.yaml", init::WORKSPACE_YAML),
        (".gitignore", ".keel-state/\n.env\n"),
        (".env", init::ENV_TEMPLATE),
        ("global/README.md", init::GLOBAL_README),
        ("global/rules/README.md", init::GLOBAL_RULES_README),
        ("global/rules/rule.yaml.example", init::GLOBAL_RULE),
        ("global/exceptions/README.md", init::EXCEPTIONS_README),
        (
            "global/exceptions/exception.yaml.example",
            init::EXCEPTION_TMPL,
        ),
        ("global/containment/README.md", init::CONTAINMENT_README),
        (
            "global/containment/containment.yaml.example",
            init::CONTAINMENT_TMPL,
        ),
        ("organizations/README.md", init::ORGS_README),
        (
            "organizations/my-company/README.md",
            init::ORG_INSTANCE_README,
        ),
        (
            "organizations/my-company/organization.yaml.example",
            init::ORGANIZATION_TMPL,
        ),
        (
            "organizations/my-company/repositories.yaml.example",
            init::REPOSITORIES_TMPL,
        ),
        (
            "organizations/my-company/composition.yaml.example",
            init::COMPOSITION_TMPL,
        ),
        (
            "organizations/my-company/components/README.md",
            init::COMPONENTS_README,
        ),
        ("platforms/README.md", init::PLATFORMS_README),
        ("projects/README.md", init::PROJECTS_README),
        ("projects/app/README.md", init::PROJECT_INSTANCE_README),
        ("projects/app/rules/README.md", init::PROJECT_RULES_README),
        ("projects/app/rules/rule.yaml.example", init::PROJECT_RULE),
        ("projects/app/tools/README.md", init::TOOLS_README),
        ("projects/app/tools/tool.yaml.example", init::TOOL_TMPL),
        (
            "projects/app/skills/README.md",
            init::GOVERNED_RESOURCE_README,
        ),
        (
            "projects/app/knowledge/README.md",
            init::GOVERNED_RESOURCE_README,
        ),
        (
            "projects/app/workflows/README.md",
            init::GOVERNED_RESOURCE_README,
        ),
        (
            "projects/app/workflows/default.yaml",
            init::DEFAULT_WORKFLOW,
        ),
        (
            "projects/app/contracts/README.md",
            init::GOVERNED_RESOURCE_README,
        ),
        (
            "projects/app/agents/README.md",
            init::GOVERNED_RESOURCE_README,
        ),
        (
            "projects/app/hooks/README.md",
            init::GOVERNED_RESOURCE_README,
        ),
        (
            "projects/app/providers/README.md",
            init::GOVERNED_RESOURCE_README,
        ),
        (
            "projects/app/policies/README.md",
            init::GOVERNED_RESOURCE_README,
        ),
        (
            "projects/app/executors/README.md",
            init::GOVERNED_RESOURCE_README,
        ),
        ("projects/app/executors/mock.yaml", init::MOCK_EXECUTOR),
        ("projects/app/tests/README.md", init::TESTS_README),
        ("projects/app/tests/test.yaml.example", init::TEST_TMPL),
        ("teams/README.md", init::TEAMS_README),
        ("profiles/README.md", init::PROFILES_README),
        ("profiles/profile.yaml.example", init::PROFILE_TMPL),
        ("packages/README.md", init::PACKAGES_README),
        ("schemas/README.md", init::SCHEMAS_README),
        ("registry/README.md", init::REGISTRY_README),
        ("locks/README.md", init::LOCKS_README),
        ("migrations/README.md", init::MIGRATIONS_README),
        ("tests/README.md", init::WS_TESTS_README),
    ];
    for (rel, content) in files {
        let dest = path.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, content)?;
    }

    // Binding (section 8.6): which project this workspace resolves to, so
    // `keel compile` composes global + projects/app out of the box.
    ProjectBinding {
        project: "project:local/app".to_string(),
        workspace: "org:local".to_string(),
        platforms: Vec::new(),
    }
    .write(path)?;

    println!("workspace created at {}", path.display());
    println!(
        "the full section 8.5 layout, each folder with a README + a base template; bound to project:local/app"
    );
    println!(
        "active: rules, skills, knowledge, workflows, contracts, agents, hooks, providers, policies and executors"
    );
    println!(
        "verify: keel doctor --workspace {} --governed",
        path.display()
    );
    println!("launch: keel claude   (or: keel codex / keel opencode)");
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
    let outcome = compiler::compile_layered(root, &chain, now_ts())?;

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
            "  (no active rules yet — activate a template, e.g. global/rules/rule.yaml.example → global/rules/<name>.yaml)"
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
        LayerId::Package => "package",
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
    // The conversion itself lives on `Evaluation` (keel-engine) so the host
    // broker shares it; this wrapper keeps the CLI call sites unchanged.
    eval.to_ledger_entry(event, &snapshot.hash.to_string(), id, ts)
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
    use keel_engine::lock::ProjectBinding;
    use keel_engine::workspace::{Layer, load_layered};

    let layered = load_layered(root)?;
    let binding = ProjectBinding::load(root).ok();
    let selected: Vec<&Layer> = match &binding {
        Some(b) => {
            let chain = keel_engine::resolution::resolve(&layered, b, None)?;
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
    let outcome = compiler::compile_layered(root, &chain, now_ts())?;
    let tests: Vec<_> = selected
        .iter()
        .flat_map(|l| l.files.tests.iter().cloned())
        .collect();
    let reports = testkit::run_tests(&outcome.snapshot, &tests, root);

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
            loaded_skills: vec![],
            recorded_evidence: vec![],
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
        platforms: Vec::new(),
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

    let lock = Lock::generate(
        &binding,
        &snapshot,
        keel_version,
        knowledge_checkpoints(root, &snapshot),
    );
    lock.write(root)?;
    println!("locked {} @ {}", binding.project, lock.snapshot_hash);
    println!("wrote {}", Lock::path(root).display());
    Ok(ExitCode::SUCCESS)
}

// ─────────────────────────── knowledge (hash-chained growth) ───────────────────────────

/// Head entry hash of every declared `Knowledge` component's chain, at THIS
/// moment — the external witness `Lock::knowledge_checkpoints` versions.
/// Best-effort by design (mirrors `Ledger::recorded_evidence`'s posture
/// elsewhere): a component with no chain file yet (nothing appended so far)
/// or an unreadable one is simply absent from the map, never a `keel lock`
/// failure — locking a workspace before any knowledge has grown is normal.
fn knowledge_checkpoints(
    root: &Path,
    snapshot: &Snapshot,
) -> std::collections::BTreeMap<String, String> {
    snapshot
        .components
        .values()
        .filter(|c| c.kind == "knowledge")
        .filter_map(|c| {
            let path = knowledge_chain_path(root, snapshot, &c.id).ok()?;
            let head = KnowledgeChain::open(&path).ok()?.head(&c.id).ok()??;
            Some((format!("knowledge:{}", c.id), head.entry_hash))
        })
        .collect()
}

/// Resolves a `Knowledge` component's chain database path from the compiled
/// snapshot's `spec.content` — the SAME field `keel lock` already treats as
/// a path, not hashed content (`snapshot.rs`: "paths only"). A `Knowledge`
/// component with no `content` (e.g. only `inline`) is a workspace-authoring
/// error for this purpose: the chain needs a durable file to grow into.
fn knowledge_chain_path(root: &Path, snapshot: &Snapshot, id: &str) -> Result<std::path::PathBuf> {
    // NOTE: component kind keys are lowercase in the compiled snapshot
    // (`workspace.rs` pushes `("knowledge".to_string(), ...)`, `compile.rs`
    // builds the key as `format!("{kind}:{id}")`) — NOT `Knowledge:<id>`.
    let key = format!("knowledge:{id}");
    let component = snapshot.components.get(&key).with_context(|| {
        format!("no `Knowledge` component `{id}` in the compiled snapshot (looked for `{key}`)")
    })?;
    let rel = component.content.as_deref().with_context(|| {
        format!("Knowledge `{id}` has no `spec.content` (chain path) — `spec.inline` is not supported here")
    })?;
    Ok(root.join(rel))
}

/// `keel knowledge append` — writes one entry to a `Knowledge` component's
/// hash chain (spec-adjacent: see `keel-runtime::knowledge_chain`). Never
/// touches the snapshot hash: this is deliberately a different plane than
/// `keel compile`/`keel lock`.
pub(crate) fn knowledge_append(
    root: &Path,
    id: &str,
    content: &str,
    session: Option<&str>,
) -> Result<ExitCode> {
    let files = workspace::load(root)?;
    let snapshot = Snapshot::load(&files.snapshot_path())
        .context("no published snapshot — run `keel compile` first")?;
    let path = knowledge_chain_path(root, &snapshot, id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let chain = KnowledgeChain::open(&path)
        .with_context(|| format!("could not open knowledge chain at {}", path.display()))?;
    let entry = chain.append(id, session, content)?;
    println!("appended {} to `{id}` (seq {})", entry.entry_id, entry.seq);
    println!("entry_hash: {}", entry.entry_hash);
    Ok(ExitCode::SUCCESS)
}

/// `keel knowledge verify` — recomputes a `Knowledge` component's chain from
/// storage and reports where it broke, if ever. Without `id`, verifies every
/// `Knowledge` component declared in the snapshot. This is integrity
/// verification (was any past entry rewritten), NOT drift detection (that
/// remains `keel lock --verify`'s job) — the two are deliberately separate.
pub(crate) fn knowledge_verify(root: &Path, id: Option<&str>) -> Result<ExitCode> {
    let files = workspace::load(root)?;
    let snapshot = Snapshot::load(&files.snapshot_path())
        .context("no published snapshot — run `keel compile` first")?;

    let ids: Vec<String> = match id {
        Some(id) => vec![id.to_string()],
        None => snapshot
            .components
            .values()
            .filter(|c| c.kind == "knowledge")
            .map(|c| c.id.clone())
            .collect(),
    };
    if ids.is_empty() {
        println!("no `Knowledge` components declared in the snapshot.");
        return Ok(ExitCode::SUCCESS);
    }

    let mut any_broken = false;
    for id in &ids {
        let path = knowledge_chain_path(root, &snapshot, id)?;
        let chain = KnowledgeChain::open(&path)
            .with_context(|| format!("could not open knowledge chain at {}", path.display()))?;
        let report = chain.verify_chain(id)?;
        match report.broken_at {
            None => println!(
                "`{id}`: OK — {} entries, chain intact.",
                report.verified_entries
            ),
            Some(broken) => {
                any_broken = true;
                println!(
                    "`{id}`: BROKEN at seq {} (entry {}) — {} entries verified before the break.",
                    broken.seq, broken.entry_id, report.verified_entries
                );
                println!("  expected: {}", broken.expected_hash);
                println!("  found:    {}", broken.found_hash);
            }
        }
    }
    Ok(if any_broken {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
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
