// SPDX-License-Identifier: Apache-2.0
//! The hard ring — OS sandbox backstop (spec section 5.2 runner).
//!
//! Command interposition (shims) governs the PATH surface; a determined
//! child can step around it with an absolute path or a reset PATH. The OS
//! sandbox is the ring the child cannot step around: the kernel refuses the
//! action regardless of how it was invoked.
//!
//! A [`SandboxProvider`] turns the compiled [`CompiledContainment`] into a
//! platform confinement wrapper. macOS uses Seatbelt (`sandbox-exec`); Linux
//! (Landlock) is a later provider behind the same trait. Availability is
//! probed, never assumed: when no provider can honor the containment the
//! launcher degrades to shims-only WITH A BANNER — a silent downgrade of a
//! security boundary is worse than a loud one.

use keel_engine::snapshot::CompiledContainment;
use std::path::Path;

/// A platform confinement wrapper around the child command.
pub trait SandboxProvider {
    /// Human name for banners/doctor.
    fn name(&self) -> &'static str;

    /// Whether this provider can actually run here (binary present, kernel
    /// feature available). Probed at launch and by `keel doctor`.
    fn available(&self) -> bool;

    /// Wraps `argv` so it runs confined by `containment`, rooted at
    /// `workspace`. Returns the full argv to spawn (wrapper + original).
    fn wrap(
        &self,
        argv: &[String],
        containment: &CompiledContainment,
        workspace: &Path,
    ) -> Vec<String>;
}

/// The provider for the current platform, if any.
pub fn provider() -> Option<Box<dyn SandboxProvider>> {
    #[cfg(target_os = "macos")]
    {
        Some(Box::new(seatbelt::Seatbelt))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// The containment level actually in effect for a session — reported in the
/// banner so the operator always knows which ring is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Shims + OS sandbox (the hard ring is active).
    Full,
    /// Shims only (no sandbox provider available, or none requested).
    Shims,
}

#[cfg(target_os = "macos")]
pub mod seatbelt {
    //! macOS Seatbelt profile (SBPL) generation.
    //!
    //! `sandbox-exec` is deprecated at the CLI surface but is the same
    //! mechanism Bazel, Nix and Chromium rely on today; the underlying
    //! `sandbox_init`/SBPL API is what macOS uses internally. We generate an
    //! allow-by-default profile with TARGETED denies so the child CLI keeps
    //! working, and probe availability so a future macOS that drops it
    //! degrades loudly instead of failing shut on every command.

    use super::SandboxProvider;
    use keel_engine::snapshot::CompiledContainment;
    use std::path::Path;

    pub struct Seatbelt;

    impl SandboxProvider for Seatbelt {
        fn name(&self) -> &'static str {
            "seatbelt"
        }

        fn available(&self) -> bool {
            // A trivial allow-all profile that must exit 0. If sandbox-exec is
            // missing or the kernel refuses the profile, we are not available.
            std::process::Command::new("sandbox-exec")
                .args(["-p", "(version 1)(allow default)", "true"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }

        fn wrap(
            &self,
            argv: &[String],
            containment: &CompiledContainment,
            workspace: &Path,
        ) -> Vec<String> {
            let profile = profile(containment, workspace);
            let mut wrapped = vec!["sandbox-exec".to_string(), "-p".to_string(), profile];
            wrapped.extend(argv.iter().cloned());
            wrapped
        }
    }

    /// Builds the SBPL profile: allow-by-default, then the containment's denies.
    /// Deny rules win over allows in SBPL, so ordering is not load-bearing, but
    /// we keep the denies last for readability.
    pub fn profile(containment: &CompiledContainment, workspace: &Path) -> String {
        let mut p = String::from("(version 1)\n(allow default)\n");

        // Protect keel's enforcement-critical CONFIG from the child (so it
        // cannot remove the hook, swap the shims, or tamper the rules): the
        // session host dir (ephemeral hook settings + shim scripts), the
        // published snapshot, and the binding/lock. NOT the whole `.keel-state`
        // — the ledger and the runtime/scheduler SQLite DBs must stay writable,
        // because keel's OWN children (`keel gate`, `keel mcp`) run inside this
        // same sandbox and need to append evidence and skill receipts. Deny
        // wins over allow in SBPL, so these hold even inside the workspace.
        // Directories → subpath; single files → literal (SBPL distinguishes).
        for dir in [
            workspace.join(".keel-state").join("host"),
            workspace.join(".keel"),
        ] {
            p.push_str(&format!(
                "(deny file-write* (subpath \"{}\"))\n",
                sbpl_escape(&dir.display().to_string())
            ));
        }
        for file in [
            workspace.join(".keel-state").join("snapshot.json"),
            workspace.join(".keel-state").join("snapshot.prev.json"),
        ] {
            p.push_str(&format!(
                "(deny file-write* (literal \"{}\"))\n",
                sbpl_escape(&file.display().to_string())
            ));
        }

        // Deny deletion of matching files anywhere (file-write-unlink is the
        // op behind rm/unlink). We match on the basename glob translated to a
        // regex, so `**/*.md` and `*.md` both mean "any .md file".
        for glob in &containment.deny_unlink {
            let re = glob_to_regex(glob);
            p.push_str(&format!("(deny file-write-unlink (regex #\"{re}\"))\n"));
        }

        // Confine writes to the workspace subtree.
        if containment.deny_write_outside {
            let root = workspace.display().to_string();
            p.push_str("(deny file-write*)\n");
            p.push_str(&format!(
                "(allow file-write* (subpath \"{}\"))\n",
                sbpl_escape(&root)
            ));
            // Writable temp + tty so the child CLI and shells still work.
            p.push_str("(allow file-write* (subpath \"/private/tmp\"))\n");
            p.push_str("(allow file-write* (subpath \"/private/var/folders\"))\n");
            p.push_str("(allow file-write* (regex #\"^/dev/tty\"))\n");
        }

        if containment.deny_network {
            p.push_str("(deny network*)\n");
        }

        p
    }

    /// Escapes a path for an SBPL string literal.
    fn sbpl_escape(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }

    /// Translates a path glob into an SBPL regex that matches the whole path.
    /// `*` → any run of non-slash chars, `**` → any run including slashes,
    /// `.` is literal. Anchored at the end so `**/*.md` matches `/a/b/x.md`.
    fn glob_to_regex(glob: &str) -> String {
        let mut re = String::from("");
        let bytes = glob.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] as char {
                '*' => {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                        re.push_str(".*");
                        i += 2;
                        // Skip a following slash so `**/x` also matches `x`.
                        if i < bytes.len() && bytes[i] == b'/' {
                            re.push_str("/?");
                            i += 1;
                        }
                        continue;
                    }
                    re.push_str("[^/]*");
                }
                '.' => re.push_str("\\."),
                '\\' => re.push_str("\\\\"),
                '"' => re.push_str("\\\""),
                c => re.push(c),
            }
            i += 1;
        }
        // Match the full path ending in the glob.
        format!("{re}$")
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "../tests-unit/sandbox_seatbelt.rs"]
mod seatbelt_tests;
