// SPDX-License-Identifier: Apache-2.0
//! `keel init` and `keel doctor` — bootstrapping and verifying a governed
//! workspace.
//!
//! Keel does NOT drive model provider APIs (D-012): there is no `keel run`
//! session, no provider/credential configuration and no keychain here. A
//! model runs under `keel <cli>` (the parent runtime, `keel-host`); governed
//! agents run as local CLI executors resolved from the snapshot. These two
//! commands only prepare and check the durable artifacts an operator needs
//! before launching.

use crate::commands;
use anyhow::{Result, bail};
use keel_engine::lock::Lock;
use keel_engine::snapshot::Snapshot;
use keel_engine::workspace::WorkspaceFiles;
use keel_runtime::RuntimeStore;
use serde_json::json;
use std::path::Path;
use std::process::ExitCode;

const RUNTIME_DB: &str = "runtime.sqlite";

/// `keel init` — scaffold the workspace, compile it, pin the lock and open the
/// durable store, so the workspace is ready to `keel compile`/`keel <cli>`
/// with no manual editing in between.
pub fn init(root: &Path, json_output: bool) -> Result<ExitCode> {
    commands::init(root)?;
    commands::compile(root)?;
    commands::lock(root, false)?;
    RuntimeStore::open(&state_dir(root).join(RUNTIME_DB))?;

    // Register it as the operator's default so `keel <cli>` finds it from
    // anywhere without --workspace. Best-effort: a config write failure must
    // not fail the init (the workspace is already good).
    if let Err(e) = keel_host::config::set_default_workspace(root) {
        eprintln!("[keel] note: could not register default workspace: {e}");
    }

    emit(
        json!({
            "status": "ready",
            "workspace": root,
        }),
        json_output,
    );
    Ok(ExitCode::SUCCESS)
}

/// `keel use <workspace>` — register the default workspace for `keel <cli>`.
pub fn use_workspace(root: &Path, json_output: bool) -> Result<ExitCode> {
    // Only accept an actual workspace, so a typo does not silently become the
    // default.
    if !root.join("workspace.yaml").exists() {
        bail!(
            "`{}` is not a keel workspace (no workspace.yaml)",
            root.display()
        );
    }
    keel_host::config::set_default_workspace(root)?;
    emit(
        json!({ "status": "ok", "default_workspace": root }),
        json_output,
    );
    Ok(ExitCode::SUCCESS)
}

/// `keel doctor --governed` — verify the governed baseline: the snapshot
/// loads and self-verifies, the lock matches the published snapshot, and the
/// durable store opens.
pub fn doctor(root: &Path, json_output: bool) -> Result<ExitCode> {
    let files = WorkspaceFiles::empty(root.to_path_buf());
    let snapshot = Snapshot::load(&files.snapshot_path())?;
    let lock = Lock::load(root)?;
    if lock.snapshot_hash != snapshot.hash.to_string() {
        bail!("lock and published snapshot differ; run `keel lock` before launching");
    }
    RuntimeStore::open(&state_dir(root).join(RUNTIME_DB))?;
    emit(
        json!({
            "status": "ready",
            "governed": true,
            "snapshot_hash": snapshot.hash,
        }),
        json_output,
    );
    Ok(ExitCode::SUCCESS)
}

fn state_dir(root: &Path) -> std::path::PathBuf {
    root.join(".keel-state")
}

fn emit(value: serde_json::Value, json_output: bool) {
    if json_output {
        println!("{}", serde_json::to_string(&value).unwrap_or_default());
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
    }
}
