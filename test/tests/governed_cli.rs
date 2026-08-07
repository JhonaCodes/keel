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
