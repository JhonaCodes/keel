// SPDX-License-Identifier: Apache-2.0
//! `keel <cli>` — assembling the governed environment and launching the child.
//!
//! Order matters and is the whole point (spec section 5.3): the snapshot is
//! loaded and pinned FIRST, the containment (shims → broker) is built SECOND,
//! and only then does the client CLI get to exist — inside it.

use crate::broker::{Broker, spawn as spawn_broker};
use crate::{pty, sandbox, shims};
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

    // The hard ring: wrap argv in the OS sandbox when the snapshot declares a
    // containment AND a provider can honor it. Any downgrade is announced.
    let (argv, level) = apply_sandbox(argv, &snapshot_containment, &root, opts.containment);

    eprintln!(
        "[keel] session {session_id} — containment: {} ({}) — workspace {}",
        level_label(level),
        manifest.id,
        root.display()
    );

    let cwd = std::env::current_dir()?;
    let code = pty::run(&argv, &env, &cwd);

    // Teardown: broker down, ephemeral dir + socket gone; evidence stays.
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

/// Workspace resolution: `--workspace` > `KEEL_WORKSPACE` > walk-up from cwd
/// looking for `workspace.yaml`.
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
            bail!(
                "no keel workspace found — pass --workspace, set KEEL_WORKSPACE, \
                 or run inside a workspace (keel init)"
            );
        }
    }
}
