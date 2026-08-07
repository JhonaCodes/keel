// SPDX-License-Identifier: Apache-2.0
//! Session shim dir — command interposition (spec section 5.3 inner ring).
//!
//! For each governed command, a generated script is placed FIRST in the
//! child's PATH. The script execs `keel-shim`, which asks the broker and only
//! then execs the REAL binary. Everything the script needs is BAKED into the
//! file at generation time (socket path, real binary path): identity does not
//! travel through environment variables the child could alter — the shim dir
//! itself is the sealed artifact (0700, owned by the parent).
//!
//! Interposition governs the PATH surface. Absolute-path invocations
//! (`/bin/rm`) bypass it by construction; that is the OS-sandbox plane's job
//! (F2), and the adapter preflight is honest about it (invariant 8).

use anyhow::{Context, Result, bail};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Generates the shim dir for a session. Returns the dir path; commands that
/// do not exist on this machine are skipped (interposing a binary that is not
/// installed would be a no-op anyway).
pub fn generate(
    host_dir: &Path,
    shim_bin: &Path,
    socket_path: &Path,
    commands: &[String],
) -> Result<PathBuf> {
    let bin_dir = host_dir.join("bin");
    std::fs::create_dir_all(&bin_dir)?;
    std::fs::set_permissions(&bin_dir, std::fs::Permissions::from_mode(0o700))?;

    for name in commands {
        let Some(real) = resolve_real(name, &bin_dir) else {
            continue;
        };
        let script = format!(
            "#!/bin/sh\n\
             # keel shim — governed command interposition (generated per session)\n\
             exec \"{shim}\" --socket \"{socket}\" --name \"{name}\" --real \"{real}\" -- \"$@\"\n",
            shim = shim_bin.display(),
            socket = socket_path.display(),
            real = real.display(),
        );
        let path = bin_dir.join(name);
        std::fs::write(&path, script)
            .with_context(|| format!("shims: could not write {}", path.display()))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(bin_dir)
}

/// Locates the `keel-shim` binary: next to the current `keel` executable.
/// The installer places both in the same prefix; a missing shim binary is a
/// broken installation, not a degraded mode (fail-closed).
pub fn shim_binary() -> Result<PathBuf> {
    let me = std::env::current_exe().context("shims: cannot resolve the current executable")?;
    let dir = me
        .parent()
        .context("shims: the current executable has no parent dir")?;
    let candidate = dir.join("keel-shim");
    if !candidate.exists() {
        bail!(
            "keel-shim not found at {} — reinstall keel (both binaries ship together)",
            candidate.display()
        );
    }
    Ok(candidate)
}

/// Resolves the REAL binary for `name` by walking PATH, excluding the shim
/// dir itself (a shim must never exec another shim).
fn resolve_real(name: &str, exclude_dir: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        if dir == exclude_dir {
            continue;
        }
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "../tests-unit/shims.rs"]
mod tests;
