// SPDX-License-Identifier: Apache-2.0
//! Hermetic test harness — black-box tests that are IMMUNE to the host's real
//! configuration and environment.
//!
//! Every `keel` subprocess is spawned with `Command::env_clear()` plus a
//! minimal allowlist, so the child sees ONLY the environment the test declares
//! — never a stray host/CI var. This closes the contamination path where an
//! inherited env var (e.g. `PROD_WRITE_ENABLED`) folded into the event at
//! `gate.rs` could silently flip an `env.present` precondition. A test's
//! outcome depends solely on what it injects, not on who runs it.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// The compiled `keel` binary, resolved like the other black-box tests
/// (`test/../target/<profile>/keel`), overridable by `KEEL_BIN`. Read in the
/// PARENT process, so `env_clear()` on the child never affects it.
pub fn keel_bin() -> PathBuf {
    if let Ok(p) = std::env::var("KEEL_BIN") {
        return PathBuf::from(p);
    }
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // keel/
    p.push("target");
    p.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    p.push("keel");
    p
}

/// An isolated, throwaway workspace under the OS temp dir. Compiled on
/// construction; removed on drop.
pub struct HermeticWs {
    root: PathBuf,
}

impl HermeticWs {
    /// Builds a workspace from `(rule_file_name, rule_yaml)` pairs and compiles
    /// it. Panics on any setup/compile failure (a test cannot proceed otherwise).
    pub fn new(rules: &[(&str, &str)]) -> HermeticWs {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("keel-hermetic-{}-{n}", std::process::id()));
        // Fresh: a crashed prior run with a recycled pid must not leak in.
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("rules")).unwrap();
        fs::write(
            root.join("workspace.yaml"),
            "apiVersion: keel/v1alpha1\nkind: Workspace\nmetadata: { id: hermetic }\nspec: {}\n",
        )
        .unwrap();
        for (name, yaml) in rules {
            fs::write(root.join("rules").join(name), yaml).unwrap();
        }
        let ws = HermeticWs { root };
        ws.compile();
        ws
    }

    fn compile(&self) {
        let out = self.run(&["compile", "--workspace", self.root_str()], None, &[]);
        assert!(
            out.status.success(),
            "hermetic compile failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Runs `keel gate` on `event` (stdin) and returns the exit code.
    ///
    /// `extra_env` is the ONLY environment the child receives beyond the PATH
    /// allowlist — use it to declare exactly the world the precondition should
    /// see. `no_inherit_env` adds `--no-inherit-env` (event-env-only semantics).
    pub fn gate(
        &self,
        session: &str,
        event: &str,
        extra_env: &[(&str, &str)],
        no_inherit_env: bool,
    ) -> i32 {
        let mut args = vec!["gate", "--workspace", self.root_str(), "--session", session];
        if no_inherit_env {
            args.push("--no-inherit-env");
        }
        let out = self.run(&args, Some(event), extra_env);
        out.status.code().expect("gate returned no exit code")
    }

    /// Also exposes the packet on stderr, for content assertions.
    pub fn gate_output(
        &self,
        session: &str,
        event: &str,
        extra_env: &[(&str, &str)],
        no_inherit_env: bool,
    ) -> (i32, String) {
        let mut args = vec!["gate", "--workspace", self.root_str(), "--session", session];
        if no_inherit_env {
            args.push("--no-inherit-env");
        }
        let out = self.run(&args, Some(event), extra_env);
        (
            out.status.code().expect("gate returned no exit code"),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn root_str(&self) -> &str {
        self.root.to_str().unwrap()
    }

    /// The hermetic spawn: cleared environment + PATH allowlist + declared vars.
    fn run(&self, args: &[&str], stdin: Option<&str>, extra_env: &[(&str, &str)]) -> Output {
        let bin = keel_bin();
        assert!(
            bin.exists(),
            "keel binary not found at {} — run via `cargo test --workspace`",
            bin.display()
        );
        let mut cmd = Command::new(&bin);
        cmd.env_clear();
        // Allowlist PATH only, so external tool resolution still works while no
        // host state leaks in. Builtin-only rules don't even need it.
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to spawn keel");
        if let Some(input) = stdin {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
        }
        child.wait_with_output().expect("failed to wait for keel")
    }
}

impl Drop for HermeticWs {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Convenience for the odd test that only needs the workspace root path.
impl AsRef<Path> for HermeticWs {
    fn as_ref(&self) -> &Path {
        &self.root
    }
}
