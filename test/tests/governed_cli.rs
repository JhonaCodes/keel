// SPDX-License-Identifier: Apache-2.0
//! Black-box contract for the governed product surface.

use keel_tests::hermetic::keel_bin;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("keel-governed-cli-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(keel_bin());
        command.env_clear();
        if let Ok(path) = std::env::var("PATH") {
            command.env("PATH", path);
        }
        command.args(args).output().expect("spawn keel")
    }

    /// Like `run`, but with an isolated `HOME` so the global default-workspace
    /// config (`~/.keel/config.json`) is written under `home`, not the
    /// developer's real one.
    fn run_with_home(&self, args: &[&str], home: &Path) -> Output {
        let mut command = Command::new(keel_bin());
        command.env_clear();
        if let Ok(path) = std::env::var("PATH") {
            command.env("PATH", path);
        }
        command.env("HOME", home);
        command.args(args).output().expect("spawn keel")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn json(output: &Output) -> Value {
    let last_line = output
        .stdout
        .split(|byte| *byte == b'\n')
        .rev()
        .find(|line| !line.is_empty())
        .unwrap_or(&output.stdout);
    serde_json::from_slice(last_line).unwrap_or_else(|error| {
        panic!(
            "expected JSON stdout: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn init_and_doctor_prepare_a_governed_baseline_without_provider_configuration() {
    let workspace = Workspace::new();
    let root = workspace.path().to_str().unwrap();

    // No `--executor`, no provider/credential config: keel does not drive
    // model APIs (D-012). init scaffolds, compiles, pins the lock and opens
    // the store.
    let init = workspace.run(&["init", root, "--json"]);
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert_eq!(json(&init)["status"], "ready");
    assert!(workspace.path().join(".keel-state/snapshot.json").exists());
    assert!(workspace.path().join(".keel/keel.lock").exists());
    assert!(workspace.path().join(".keel-state/runtime.sqlite").exists());

    let doctor = workspace.run(&["doctor", "--workspace", root, "--governed", "--json"]);
    assert!(
        doctor.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    assert_eq!(json(&doctor)["governed"], true);

    // The removed API path leaves no provider footprint on disk.
    for forbidden in [
        ".claude",
        ".agents",
        "clients",
        ".keel-state/runtime-config.json",
    ] {
        assert!(
            !workspace.path().join(forbidden).exists(),
            "init created forbidden provider path `{forbidden}`"
        );
    }
}

#[test]
fn the_removed_api_subcommands_are_gone() {
    let workspace = Workspace::new();
    let root = workspace.path().to_str().unwrap();
    assert!(workspace.run(&["init", root, "--json"]).status.success());

    // `keel run` and `keel configure` were the API-session surface — removed
    // with the provider drivers (D-012). They must not exist.
    for argv in [
        vec!["run", "--workspace", root, "--task", "x"],
        vec!["configure", "executor", "list", "--workspace", root],
    ] {
        let out = workspace.run(&argv);
        assert!(
            !out.status.success(),
            "removed subcommand `{}` still runs",
            argv[0]
        );
    }
}

/// A scratch `keel init` must NOT steal an already-registered default
/// workspace: the FIRST init registers it, a SECOND init leaves it alone, and
/// only `keel use` switches it explicitly. (Regression: a test `keel init` on a
/// temp dir once clobbered the operator's default, and deleting the temp dir
/// then broke every `keel <cli>` with "no workspace found".)
#[test]
fn init_does_not_steal_an_existing_default_workspace() {
    let home = Workspace::new();
    fs::create_dir_all(home.path()).unwrap();

    let first = Workspace::new();
    let second = Workspace::new();
    let first_root = first.path().to_str().unwrap();
    let second_root = second.path().to_str().unwrap();
    let config = home.path().join(".keel/config.json");

    // First init (no default yet) registers `first`.
    assert!(
        first
            .run_with_home(&["init", first_root, "--json"], home.path())
            .status
            .success()
    );
    let after_first = fs::read_to_string(&config).expect("config written by first init");
    assert!(
        after_first.contains(&first.path().canonicalize().unwrap().display().to_string()),
        "first init should register itself as the default: {after_first}"
    );

    // Second init (a valid default already exists) must NOT clobber it.
    assert!(
        second
            .run_with_home(&["init", second_root, "--json"], home.path())
            .status
            .success()
    );
    let after_second = fs::read_to_string(&config).unwrap();
    assert!(
        after_second.contains(&first.path().canonicalize().unwrap().display().to_string()),
        "a second init must leave the existing default alone: {after_second}"
    );
    assert!(
        !after_second.contains(&second.path().canonicalize().unwrap().display().to_string()),
        "the scratch second init must not become the default: {after_second}"
    );

    // `keel use` is the explicit way to switch.
    assert!(
        second
            .run_with_home(&["use", second_root], home.path())
            .status
            .success()
    );
    let after_use = fs::read_to_string(&config).unwrap();
    assert!(
        after_use.contains(&second.path().canonicalize().unwrap().display().to_string()),
        "keel use must switch the default explicitly: {after_use}"
    );
}

#[test]
fn doctor_rejects_a_snapshot_that_is_not_pinned_by_the_lock() {
    let workspace = Workspace::new();
    let root = workspace.path().to_str().unwrap();
    assert!(workspace.run(&["init", root, "--json"]).status.success());

    // Recompile a changed workspace WITHOUT re-locking: the published snapshot
    // now differs from the pinned lock. doctor (the pre-launch gate) refuses.
    let knowledge = workspace.path().join("projects/app/knowledge/drift.yaml");
    fs::write(
        knowledge,
        "apiVersion: keel/v1alpha1\nkind: Knowledge\nmetadata: { id: drift, version: 1.0.0 }\nspec: { inline: changed }\n",
    )
    .unwrap();
    assert!(
        workspace
            .run(&["compile", "--workspace", root])
            .status
            .success()
    );

    let doctor = workspace.run(&["doctor", "--workspace", root, "--governed", "--json"]);
    assert!(!doctor.status.success());
    assert!(String::from_utf8_lossy(&doctor.stderr).contains("lock and published snapshot differ"));
}
