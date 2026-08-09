// SPDX-License-Identifier: Apache-2.0
//! Global keel config — the operator's default workspace.
//!
//! Lives at `~/.keel/config.json`, OUTSIDE any workspace. It only records a
//! convenience default so `keel <cli>` works from anywhere after `keel init`
//! (or `keel use`). It is NOT part of the governed snapshot and never affects
//! enforcement — just workspace resolution.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
struct GlobalConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_workspace: Option<String>,
}

/// `~/.keel/config.json` (None if HOME is unset).
fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".keel").join("config.json"))
}

fn load() -> GlobalConfig {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// The operator's default workspace, if one is registered and still exists.
pub fn default_workspace() -> Option<PathBuf> {
    let path = PathBuf::from(load().default_workspace?);
    path.join("workspace.yaml").exists().then_some(path)
}

/// Records `workspace` as the default (canonicalized to an absolute path so it
/// resolves from any cwd). Overwrites the previous default — `keel use` always
/// wins; `keel init` defers to an existing valid default (see `governed::init`)
/// so a scratch workspace cannot silently steal it.
pub fn set_default_workspace(workspace: &Path) -> Result<()> {
    let path = config_path().context("cannot locate ~/.keel (HOME unset)")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let absolute = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let config = GlobalConfig {
        default_workspace: Some(absolute.display().to_string()),
    };
    std::fs::write(&path, serde_json::to_string_pretty(&config)?)
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests-unit/config.rs"]
mod tests;
