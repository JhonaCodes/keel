// SPDX-License-Identifier: Apache-2.0
//! `keel <cli>` — assembling the governed environment and launching the child.
//!
//! Order matters and is the whole point (spec section 5.3): the snapshot is
//! loaded and pinned FIRST, the containment (shims → broker) is built SECOND,
//! and only then does the client CLI get to exist — inside it.

use crate::broker::{Broker, spawn as spawn_broker};
use crate::{pty, sandbox, shims, supervisor};
use anyhow::{Context, Result, bail};
use keel_engine::adapter::AdapterManifest;
use keel_engine::ledger::Ledger;
use keel_engine::snapshot::Snapshot;
use keel_engine::workspace::WorkspaceFiles;
use keel_runtime::RuntimeStore;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::ExitCode;

const RUNTIME_DB: &str = "runtime.sqlite";

pub struct LaunchOptions {
    /// Launch adapter id (`claude`, `codex`, `generic`).
    pub client: String,
    /// Explicit workspace; otherwise `KEEL_WORKSPACE`, then walk-up from cwd.
    pub workspace: Option<PathBuf>,
    /// Extra argv appended to the adapter's base command (for `generic` it IS
    /// the command).
    pub cmd: Vec<String>,
    /// Initial task, appended as a positional argument when present.
    pub task: Option<String>,
    /// Resume a keel session identity; a fresh ulid otherwise.
    pub session: Option<String>,
    /// Requested containment level. `Full` (default) uses the OS sandbox when
    /// available; `Shims` skips it deliberately (the banner says so).
    pub containment: ContainmentMode,
    /// When true, the supervisor does not surface cognitive-direction
    /// suggestions (P3). Enforcement is unaffected.
    pub no_suggest: bool,
}

/// What the operator asked for. The EFFECTIVE level (see `sandbox::Level`) can
/// be lower — always with a banner, never silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContainmentMode {
    /// Shims + OS sandbox when a provider is available.
    #[default]
    Full,
    /// Shims only — explicit opt-out of the hard ring.
    Shims,
}

pub fn launch(opts: LaunchOptions) -> Result<ExitCode> {
    let root = resolve_workspace(opts.workspace.clone())?;
    let files = WorkspaceFiles::empty(root.clone());
    let snapshot = Snapshot::load(&files.snapshot_path())
        .context("no published snapshot — run `keel compile` first")?;

    let manifest = AdapterManifest::for_client(&opts.client).with_context(|| {
        format!(
            "unknown client `{}` — use `keel launch --client generic -- <cmd>`",
            opts.client
        )
    })?;

    // Invariant 8, out loud: a block the containment cannot honor is reported
    // BEFORE the session starts, never silently promised.
    for violation in keel_engine::adapter::preflight(&snapshot, &manifest) {
        eprintln!("[keel] preflight: {}", violation.reason);
    }

    let mut argv = manifest.command.clone();
    argv.extend(opts.cmd.iter().cloned());
    if let Some(task) = &opts.task {
        argv.push(task.clone());
    }
    if argv.is_empty() {
        bail!("client `generic` needs an explicit command: keel launch --client generic -- <cmd>");
    }

    let session_id = opts
        .session
        .clone()
        .unwrap_or_else(|| format!("session-{}", ulid::Ulid::new()));

    // Session state dir: sealed (0700), everything ephemeral lives here; the
    // ledger and the runtime store live OUTSIDE it and survive the session.
    let state_dir = files.state_dir();
    let host_dir = state_dir.join("host").join(&session_id);
    std::fs::create_dir_all(&host_dir)?;
    std::fs::set_permissions(&host_dir, std::fs::Permissions::from_mode(0o700))?;

    let store = RuntimeStore::open(&state_dir.join(RUNTIME_DB))?;
    store.ensure_session(&session_id, &snapshot.hash.to_string())?;

    // The socket lives OUTSIDE the workspace, in a short temp path: a UNIX
    // socket path must fit in SUN_LEN (~104 bytes on macOS), which a nested
    // workspace/.keel-state/host/<ulid>/ path easily overflows. It is not
    // security-bearing (it only evaluates; identity is baked into the shims),
    // so a short unique name is enough. Cleaned up on teardown.
    let socket_path = std::env::temp_dir().join(format!("keel-{}.sock", short_id(&session_id)));
    let ledger = Ledger::open(&files.ledger_path())?;
    // Capture the containment before the snapshot moves into the broker: the
    // sandbox is generated from it just below.
    let snapshot_containment = snapshot.containment.clone();
    let broker = Broker::new(snapshot, ledger, root.clone(), session_id.clone());
    let (join, handle) = spawn_broker(broker, &socket_path)?;

    let shim_bin = shims::shim_binary()?;
    let shim_dir = shims::generate(&host_dir, &shim_bin, &socket_path, &manifest.shim_commands)?;

    // The fabricated environment: interposition first on PATH; KEEL_* vars are
    // INFORMATIVE (session identity travels baked inside the shims, never
    // through env the child could rewrite).
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    env.insert(
        "PATH".into(),
        format!("{}:{}", shim_dir.display(), inherited_path),
    );
    env.insert("KEEL_SESSION".into(), session_id.clone());
    env.insert("KEEL_WORKSPACE".into(), root.display().to_string());

    // Convergence plane (P2): wire keel's MCP endpoint into the child so the
    // model discovers/loads its governed skills through keel, and announce it.
    // Ephemeral, per-session config; if the child ignores or deletes it, the
    // hard rings are unaffected (P1 never depends on P2).
    let argv = wire_convergence(argv, &manifest, &host_dir, &root, &session_id)?;

    // The hard ring: wrap argv in the OS sandbox when the snapshot declares a
    // containment AND a provider can honor it. Any downgrade is announced.
    let (argv, level) = apply_sandbox(argv, &snapshot_containment, &root, opts.containment);

    eprintln!(
        "[keel] session {session_id} — containment: {} ({}) — workspace {}",
        level_label(level),
        manifest.id,
        root.display()
    );

    // Cognitive direction (P3): a supervisor watches the live ledger and
    // surfaces suggestions (oscillation) to the operator's transcript — never
    // into the model's input stream (that would interfere with its reasoning).
    let supervisor_shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let supervisor = (!opts.no_suggest).then(|| {
        supervisor::spawn(
            files.ledger_path(),
            session_id.clone(),
            supervisor_shutdown.clone(),
        )
    });

    let cwd = std::env::current_dir()?;
    let code = pty::run(&argv, &env, &cwd);

    // Teardown: supervisor + broker down, ephemeral dir + socket gone; evidence
    // stays.
    supervisor_shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(supervisor) = supervisor {
        let _ = supervisor.join();
    }
    handle.stop();
    let _ = join.join();
    let _ = std::fs::remove_dir_all(&host_dir);
    let _ = std::fs::remove_file(&socket_path);

    let code = code?;
    Ok(if code == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(code.clamp(0, 255) as u8)
    })
}

/// Wires keel's MCP endpoint into the child per the adapter manifest and
/// returns the augmented argv. Writes an ephemeral per-session MCP config
/// under `host_dir`; appends the client's config flag and the "you are
/// governed by keel" announcement. A client with no known wiring (generic)
/// is returned unchanged — convergence is opt-in there; the hard rings hold
/// regardless.
fn wire_convergence(
    mut argv: Vec<String>,
    manifest: &AdapterManifest,
    host_dir: &std::path::Path,
    root: &std::path::Path,
    session_id: &str,
) -> Result<Vec<String>> {
    use keel_engine::adapter::{Announce, McpMethod};

    let Some(mcp) = &manifest.mcp else {
        return Ok(argv);
    };
    let keel_bin = std::env::current_exe()
        .context("convergence: cannot resolve the keel binary for the MCP endpoint")?;
    let keel_bin = keel_bin.display().to_string();
    let root_str = root.display().to_string();

    match &mcp.method {
        McpMethod::ConfigFileFlag { flag } => {
            // Claude-style: a JSON file of MCP servers.
            let config = serde_json::json!({
                "mcpServers": {
                    "keel": {
                        "command": keel_bin,
                        "args": ["mcp", "--workspace", root_str, "--session", session_id],
                    }
                }
            });
            let path = host_dir.join("mcp.json");
            std::fs::write(&path, serde_json::to_string_pretty(&config)?)?;
            argv.push(flag.clone());
            argv.push(path.display().to_string());
        }
        McpMethod::ConfigOverrideFlag { flag } => {
            // Codex-style: dotted TOML overrides.
            let args_toml =
                format!("[\"mcp\",\"--workspace\",\"{root_str}\",\"--session\",\"{session_id}\"]");
            argv.push(flag.clone());
            argv.push(format!("mcp_servers.keel.command=\"{keel_bin}\""));
            argv.push(flag.clone());
            argv.push(format!("mcp_servers.keel.args={args_toml}"));
        }
    }

    let notice = "You are running under keel, a local governance runtime. Your skills \
                  and agents are provided BY keel through MCP tools (keel.skills.list, \
                  keel.skills.load, keel.rules.query, keel.agent.invoke). At the START of \
                  any non-trivial task, call keel.skills.list; if a skill matches the task \
                  (e.g. building a website/UI, a given domain), load it with \
                  keel.skills.load and FOLLOW it before acting — do not rely on your \
                  defaults when keel offers a skill. Some actions are contained by keel \
                  and will be refused; read the block message and adjust.";
    match &mcp.announce {
        Announce::SystemPromptFlag { flag } => {
            argv.push(flag.clone());
            argv.push(notice.to_string());
        }
        Announce::PtyLine => {
            // No client flag: the notice is printed to the session so both the
            // operator and the model see it at start.
            eprintln!("[keel] {notice}");
        }
    }

    Ok(argv)
}

/// Wraps `argv` in the OS sandbox when the snapshot declares a containment
/// and a provider can honor it. Returns the (possibly wrapped) argv and the
/// EFFECTIVE level. Every downgrade from what was requested is announced —
/// a security boundary must never weaken silently.
fn apply_sandbox(
    argv: Vec<String>,
    containment: &Option<keel_engine::snapshot::CompiledContainment>,
    workspace: &std::path::Path,
    mode: ContainmentMode,
) -> (Vec<String>, sandbox::Level) {
    let Some(containment) = containment else {
        // Nothing to enforce at the kernel: shims are the only ring by design.
        return (argv, sandbox::Level::Shims);
    };
    if mode == ContainmentMode::Shims {
        eprintln!(
            "[keel] containment: shims-only by request — the OS sandbox is OFF; \
             an absolute-path command can bypass interposition"
        );
        return (argv, sandbox::Level::Shims);
    }
    match sandbox::provider() {
        Some(provider) if provider.available() => (
            provider.wrap(&argv, containment, workspace),
            sandbox::Level::Full,
        ),
        Some(provider) => {
            eprintln!(
                "[keel] containment: DEGRADED to shims — the `{}` sandbox is not \
                 available here; an absolute-path command can bypass interposition",
                provider.name()
            );
            (argv, sandbox::Level::Shims)
        }
        None => {
            eprintln!(
                "[keel] containment: DEGRADED to shims — no OS sandbox provider on \
                 this platform yet; an absolute-path command can bypass interposition"
            );
            (argv, sandbox::Level::Shims)
        }
    }
}

fn level_label(level: sandbox::Level) -> &'static str {
    match level {
        sandbox::Level::Full => "shims + os-sandbox",
        sandbox::Level::Shims => "shims",
    }
}

/// The tail of the session ulid — enough to keep the socket name unique and
/// short (SUN_LEN budget), without exposing the full session identity.
fn short_id(session_id: &str) -> String {
    let tail: String = session_id.chars().rev().take(12).collect();
    tail.chars().rev().collect()
}

/// Workspace resolution: `--workspace` > `KEEL_WORKSPACE` > walk-up from cwd >
/// the operator's registered default (`~/.keel/config.json`, set by `keel
/// init`/`keel use`). Only then an error — so `keel <cli>` works from anywhere
/// after an init.
fn resolve_workspace(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Ok(env_ws) = std::env::var("KEEL_WORKSPACE") {
        return Ok(PathBuf::from(env_ws));
    }
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("workspace.yaml").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    if let Some(default) = crate::config::default_workspace() {
        return Ok(default);
    }
    bail!(
        "no keel workspace found — pass --workspace, set KEEL_WORKSPACE, run inside \
         a workspace, or register one with `keel init` / `keel use <path>`"
    )
}
