// SPDX-License-Identifier: Apache-2.0
//! keel — Phase 0 CLI (passive).
//!
//! This binary contains NO business logic: it parses arguments, orchestrates
//! the keel-engine modules, and presents results. It is the only crate that
//! knows about all the others (plan: "keel-cli contains no logic").
//!
//! `compile` orchestration (spec §10.2, atomic compilation):
//!   compile to staging → run RuleTests → publish ONLY if they pass,
//!   retaining the last-known-good.
//!
//! `observe` is the workhorse of Phase 0b (ADR-021): it evaluates events
//! in passive mode — every effective decision forced to `review`, nothing
//! blocks, everything is recorded in the ledger.

mod commands;
mod gate;

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
    long_about = "Keel compiles declarative constraints to an immutable snapshot and evaluates \
                  agent events outside the model. `keel gate` blocks a violating action before it \
                  runs (§5.3); `keel observe` records passively (telemetry, ADR-021)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Workspace operations.
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
    /// Compiles the workspace into an immutable snapshot (atomic: staging →
    /// RuleTests → publish only if they pass; retains last-known-good).
    Compile {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
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
    /// origin, decisions (traceability, spec §11.1).
    Explain {
        /// Evidence id (ev_…).
        ev_id: String,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        /// Emits the evidence as SARIF (ADR-016).
        #[arg(long)]
        sarif: bool,
    },
    /// Lifecycle telemetry (spec §7.7): for each rule with an expired
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
    /// (Phase 0a functional equivalence, §15.1).
    Test {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Read-only end-to-end checks of the workspace and its state.
    Doctor {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Pre-action gate (inner ring, spec §5.3): ONE event via stdin, evaluated
    /// in Enforce mode. Exit 2 = blocked (packet on stderr) — the client must
    /// not run the action. Exit 0 = allowed.
    Gate {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        /// Input protocol: omit for a native Keel event; `claude-code` parses
        /// the Claude Code hook payload (PreToolUse/PostToolUse/Stop).
        #[arg(long)]
        client: Option<String>,
        /// Session id for ledger entries (hooks usually provide their own).
        #[arg(long)]
        session: Option<String>,
        /// Shadow mode: evaluate and record, never block (try new rules risk-free).
        #[arg(long)]
        passive: bool,
    },
    /// Invoke a specialized agent (spec §14) on some material. Records an
    /// advisory semantic verdict (§6.4/§4.7) — findings, never a block.
    Audit {
        /// Agent id declared in agents/.
        #[arg(long)]
        agent: String,
        /// File with the material to analyze (diff, source, etc.).
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        session: Option<String>,
    },
    /// Client adapter helpers (thin bridges — the rules live in the runtime, §12.2).
    Adapter {
        /// Client name (supported: claude-code).
        client: String,
        /// Print the settings wiring for the client.
        #[arg(long)]
        print: bool,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Bind this repository to a project/workspace, writing `.keel/project.yaml`
    /// (§8.6, invariant 4: the repo holds only binding + lock, never the
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
    /// Generate or verify `.keel/keel.lock` — the pinned resolution (§8.6,
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
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    /// Creates the minimal scaffolding of an Keel workspace.
    Init { path: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Workspace {
            command: WorkspaceCommand::Init { path },
        } => commands::workspace_init(&path),
        Command::Compile { workspace } => commands::compile(&workspace),
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
        Command::Doctor { workspace } => commands::doctor(&workspace),
        Command::Gate {
            workspace,
            client,
            session,
            passive,
        } => {
            let client = match client.as_deref() {
                None => gate::Client::Native,
                Some("claude-code") => gate::Client::ClaudeCode,
                Some(other) => {
                    eprintln!("error: unsupported gate client `{other}` (supported: claude-code)");
                    return ExitCode::FAILURE;
                }
            };
            gate::gate(&workspace, client, session, passive)
        }
        Command::Audit {
            agent,
            input,
            workspace,
            session,
        } => gate::audit(&workspace, &agent, &input, session),
        Command::Adapter {
            client,
            print: _,
            workspace,
        } => match client.as_str() {
            "claude-code" => gate::adapter_print_claude_code(&workspace),
            other => {
                eprintln!("error: unsupported adapter `{other}` (supported: claude-code)");
                return ExitCode::FAILURE;
            }
        },
        Command::Bind {
            workspace,
            project,
            org,
        } => commands::bind(&workspace, project, org),
        Command::Lock { workspace, verify } => commands::lock(&workspace, verify),
    };

    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}