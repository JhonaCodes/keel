// SPDX-License-Identifier: Apache-2.0
//! keel — governed cognitive runtime CLI.
//!
//! This binary contains NO business logic: it parses arguments, orchestrates
//! the keel-engine modules, and presents results. It is the only crate that
//! knows about all the others (plan: "keel-cli contains no logic").
//!
//! `compile` orchestration (spec section 10.2, atomic compilation):
//!   compile to staging → run RuleTests → publish ONLY if they pass,
//!   retaining the last-known-good.
//!
//! `observe` is the workhorse of Phase 0b (ADR-021): it evaluates events
//! in passive mode — every effective decision forced to `review`, nothing
//! blocks, everything is recorded in the ledger.

mod commands;
mod governed;
mod init;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

/// Long `--version` string: carries attribution (Apache-2.0 NOTICE echoed at
/// runtime so the author travels with the tool, not only with the source).
const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nKeel — Runtime of the Agentic Cognitive Cycle (RCCA) — Copyright 2026 JhonaCodes",
    "\nApache-2.0. \"Keel\" is a trademark of JhonaCodes (see TRADEMARK.md)."
);

#[derive(Parser)]
#[command(
    name = "keel",
    version,
    long_version = LONG_VERSION,
    about = "Keel — agentic cognitive cycle runtime that holds the line on AI-agent actions",
    long_about = "Keel owns model sessions, resolves declarative context and capabilities from an \
                  immutable snapshot, validates phase artifacts and records durable evidence."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffolds a new Keel workspace with its composition layers (spec
    /// section 8.5): `global/` (applies to every project), `projects/<name>/`,
    /// plus `exceptions/`, `tools/`, `tests/` and ready-to-edit templates and a
    /// binding, so `keel compile` works immediately. Defaults to `keel-workspace`.
    Init {
        #[arg(default_value = "keel-workspace")]
        path: PathBuf,
        #[arg(long, default_value = "mock")]
        executor: String,
        #[arg(long)]
        json: bool,
    },
    /// Compiles the workspace into an immutable snapshot (atomic: staging →
    /// RuleTests → publish only if they pass; retains last-known-good).
    Compile {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Launches a client CLI as a governed CHILD of keel (parent runtime,
    /// spec section 5.3): PTY passthrough + command interposition — a blocked
    /// command never exists as a process. `keel claude`/`keel codex` are
    /// shorthands for known clients.
    Launch {
        /// Launch adapter: `claude`, `codex`, or `generic` (explicit command
        /// after `--`).
        #[arg(long)]
        client: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Initial task, passed to the client as a positional argument.
        #[arg(long)]
        task: Option<String>,
        /// Resume a keel session identity.
        #[arg(long)]
        session: Option<String>,
        /// Command (for `generic`) or extra args appended to the client CLI.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        cmd: Vec<String>,
    },
    /// `keel claude [args...]` / `keel codex [args...]` — shorthand for
    /// `keel launch --client <name> -- [args...]`.
    #[command(external_subcommand)]
    Client(Vec<String>),
    /// Evaluates events (JSONL via stdin or --events) in PASSIVE MODE and
    /// records them in the ledger. Nothing blocks (Phase 0b).
    Observe {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        /// JSONL events file; without it, reads stdin.
        #[arg(long)]
        events: Option<PathBuf>,
        /// Session identifier for the ledger entries.
        #[arg(long)]
        session: Option<String>,
    },
    /// Resolves an evidence entry: rule, version, snapshot, verdict,
    /// origin, decisions (traceability, spec section 11.1).
    Explain {
        /// Evidence id (ev_…).
        ev_id: String,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        /// Emits the evidence as SARIF (ADR-016).
        #[arg(long)]
        sarif: bool,
    },
    /// Lifecycle telemetry (spec section 7.7): for each rule with an expired
    /// reviewAfter it proposes keep/adjust/prune BACKED BY DATA. Deleting is
    /// a human decision: --record logs it with class `human`.
    Prune {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        /// Records the human decision for a rule (requires --decision and --by).
        #[arg(long)]
        record: Option<String>,
        /// keep | adjust | prune
        #[arg(long)]
        decision: Option<String>,
        /// Who decides.
        #[arg(long)]
        by: Option<String>,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Runs the workspace RuleTests against a staged snapshot
    /// (Phase 0a functional equivalence, section 15.1).
    Test {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Read-only end-to-end checks of the workspace and its state.
    Doctor {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        governed: bool,
        #[arg(long)]
        json: bool,
    },
    /// Runs or resumes a Keel-owned governed model session.
    Run {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long, conflicts_with = "resume", required_unless_present = "resume")]
        task: Option<String>,
        #[arg(long, conflicts_with = "task")]
        resume: Option<String>,
        #[arg(long)]
        executor: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Manages runtime-owned executor configuration and credentials.
    Configure {
        #[command(subcommand)]
        command: ConfigureCommand,
    },
    /// Bind this repository to a project/workspace, writing `.keel/project.yaml`
    /// (section 8.6, invariant 4: the repo holds only binding + lock, never the
    /// component definitions).
    Bind {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        /// Project identity, e.g. `project:org/repo`. If omitted, derived from
        /// the git `origin` remote.
        #[arg(long)]
        project: Option<String>,
        /// Workspace reference that owns the definitions (default `org:local`).
        #[arg(long)]
        org: Option<String>,
    },
    /// Generate or verify `.keel/keel.lock` — the pinned resolution (section 8.6,
    /// invariant 9: local and CI share the same hash). `--verify` recompiles
    /// and fails on drift (the compliance-plane check reused by CI).
    Lock {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        /// Verify the existing lock against the current snapshot instead of
        /// (re)writing it. Exit 1 on drift.
        #[arg(long)]
        verify: bool,
    },
    /// Compliance plane (spec section 5.2, section 8): the SAME engine run in CI over the
    /// pinned lock. Where `locked` finally becomes a guarantee, because CI runs
    /// on infrastructure the developer does not control.
    Ci {
        #[command(subcommand)]
        command: CiCommand,
    },
}

#[derive(Subcommand)]
enum ConfigureCommand {
    Executor {
        #[command(subcommand)]
        command: ExecutorCommand,
    },
}

#[derive(Subcommand)]
enum ExecutorCommand {
    Add {
        id: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long)]
        credential_env: Option<String>,
        #[arg(long)]
        api_key_stdin: bool,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        json: bool,
    },
    List {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Test {
        id: String,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Remove {
        id: String,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Default {
        id: String,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum CiCommand {
    /// Fail BEFORE doing work unless the binding + lock resolve: binding
    /// present, workspace compiles + RuleTests pass, and the lock matches a
    /// fresh compile (no drift). Exit non-zero on any failure.
    Resolve {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Resolve, then run the configured audit and point at the evidence. In
    /// this minimal plane `resolve` is the gate; `run` reports the ledger.
    Run {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Init {
            path,
            executor,
            json,
        } => governed::init(&path, &executor, json),
        Command::Compile { workspace } => commands::compile(&workspace),
        Command::Launch {
            client,
            workspace,
            task,
            session,
            cmd,
        } => keel_host::launch(keel_host::LaunchOptions {
            client,
            workspace,
            cmd,
            task,
            session,
        }),
        Command::Client(argv) => {
            // `keel <known-cli> [args...]`. Unknown first tokens are a hard
            // error with the generic escape hatch — never a silent guess.
            let client = argv.first().cloned().unwrap_or_default();
            if keel_engine::adapter::AdapterManifest::for_client(&client).is_none()
                || client == "generic"
            {
                Err(anyhow::anyhow!(
                    "unknown client `{client}` — use `keel launch --client generic -- <cmd>`"
                ))
            } else {
                keel_host::launch(keel_host::LaunchOptions {
                    client,
                    workspace: None,
                    cmd: argv.into_iter().skip(1).collect(),
                    task: None,
                    session: None,
                })
            }
        }
        Command::Observe {
            workspace,
            events,
            session,
        } => commands::observe(&workspace, events.as_deref(), session),
        Command::Explain {
            ev_id,
            workspace,
            sarif,
        } => commands::explain(&workspace, &ev_id, sarif),
        Command::Prune {
            workspace,
            record,
            decision,
            by,
            reason,
        } => commands::prune(&workspace, record, decision, by, reason),
        Command::Test { workspace } => commands::test(&workspace),
        Command::Doctor {
            workspace,
            governed,
            json,
        } => {
            if governed {
                governed::doctor(&workspace, json)
            } else {
                commands::doctor(&workspace)
            }
        }
        Command::Run {
            workspace,
            task,
            resume,
            executor,
            json,
        } => governed::run(
            &workspace,
            task.as_deref(),
            resume.as_deref(),
            executor.as_deref(),
            json,
        ),
        Command::Configure { command } => match command {
            ConfigureCommand::Executor { command } => match command {
                ExecutorCommand::Add {
                    id,
                    provider,
                    model,
                    endpoint,
                    credential_env,
                    api_key_stdin,
                    workspace,
                    json,
                } => governed::configure_executor_add(
                    &workspace,
                    governed::ExecutorConfiguration {
                        id,
                        provider,
                        model,
                        endpoint,
                        credential_env,
                        api_key_stdin,
                        json_output: json,
                    },
                ),
                ExecutorCommand::List { workspace, json } => {
                    governed::configure_executor_list(&workspace, json)
                }
                ExecutorCommand::Test {
                    id,
                    workspace,
                    json,
                } => governed::configure_executor_test(&workspace, &id, json),
                ExecutorCommand::Remove {
                    id,
                    workspace,
                    json,
                } => governed::configure_executor_remove(&workspace, &id, json),
                ExecutorCommand::Default {
                    id,
                    workspace,
                    json,
                } => governed::configure_executor_default(&workspace, &id, json),
            },
        },
        Command::Bind {
            workspace,
            project,
            org,
        } => commands::bind(&workspace, project, org),
        Command::Lock { workspace, verify } => commands::lock(&workspace, verify),
        Command::Ci { command } => match command {
            CiCommand::Resolve { workspace } => commands::ci_resolve(&workspace),
            CiCommand::Run { workspace } => commands::ci_run(&workspace),
        },
    };

    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
