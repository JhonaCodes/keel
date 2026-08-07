// SPDX-License-Identifier: Apache-2.0
//! Black-box contract for the governed product surface.

use keel_tests::hermetic::keel_bin;
use rusqlite::{Connection, params};
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
fn init_doctor_run_and_resume_are_governed_without_provider_configuration() {
    let workspace = Workspace::new();
    let root = workspace.path().to_str().unwrap();

    let init = workspace.run(&["init", root, "--executor", "mock", "--json"]);
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert_eq!(json(&init)["status"], "ready");
    assert!(workspace.path().join(".keel-state/snapshot.json").exists());
    assert!(workspace.path().join(".keel/keel.lock").exists());

    let doctor = workspace.run(&["doctor", "--workspace", root, "--governed", "--json"]);
    assert!(
        doctor.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    assert_eq!(json(&doctor)["governed"], true);

    let configured = workspace.run(&[
        "configure",
        "executor",
        "add",
        "mock-secondary",
        "--provider",
        "mock",
        "--model",
        "mock",
        "--workspace",
        root,
        "--json",
    ]);
    assert!(configured.status.success());
    assert_eq!(json(&configured)["status"], "configured");
    let tested = workspace.run(&[
        "configure",
        "executor",
        "test",
        "mock-secondary",
        "--workspace",
        root,
        "--json",
    ]);
    assert!(tested.status.success());
    assert_eq!(json(&tested)["status"], "ok");

    let run = workspace.run(&[
        "run",
        "--workspace",
        root,
        "--task",
        "Verify the governed runtime",
        "--executor",
        "mock",
        "--json",
    ]);
    assert!(
        run.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let result = json(&run);
    assert_eq!(result["status"], "completed");
    assert_eq!(result["phase"], "delivery");
    let session_id = result["session_id"].as_str().expect("session id");

    let resume = workspace.run(&["run", "--workspace", root, "--resume", session_id, "--json"]);
    assert!(resume.status.success());
    assert_eq!(json(&resume)["session_id"], session_id);
    assert_eq!(json(&resume)["status"], "completed");

    for forbidden in [".claude", ".agents", "clients"] {
        assert!(
            !workspace.path().join(forbidden).exists(),
            "init created forbidden provider path `{forbidden}`"
        );
    }
}

#[test]
fn resume_continues_an_interrupted_session_with_its_persisted_task_and_executor() {
    let workspace = Workspace::new();
    let root = workspace.path().to_str().unwrap();
    assert!(
        workspace
            .run(&["init", root, "--executor", "mock", "--json"])
            .status
            .success()
    );

    let snapshot: Value = serde_json::from_slice(
        &fs::read(workspace.path().join(".keel-state/snapshot.json")).unwrap(),
    )
    .unwrap();
    let snapshot_hash = snapshot["hash"].as_str().unwrap();
    let connection = Connection::open(workspace.path().join(".keel-state/runtime.sqlite")).unwrap();
    connection
        .execute(
            "INSERT INTO runtime_sessions (session_id, snapshot_hash, created_at)
             VALUES (?1, ?2, ?3)",
            params!["session-interrupted", snapshot_hash, "2026-08-07T00:00:00Z"],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO session_metadata (session_id, task, executor_id)
             VALUES (?1, ?2, ?3)",
            params!["session-interrupted", "Continue governed work", "mock"],
        )
        .unwrap();

    let resume = workspace.run(&[
        "run",
        "--workspace",
        root,
        "--resume",
        "session-interrupted",
        "--json",
    ]);
    assert!(
        resume.status.success(),
        "resume failed: {}",
        String::from_utf8_lossy(&resume.stderr)
    );
    assert_eq!(json(&resume)["status"], "completed");
    assert_eq!(json(&resume)["phase"], "delivery");
}

#[test]
fn run_rejects_a_snapshot_that_is_not_pinned_by_the_lock() {
    let workspace = Workspace::new();
    let root = workspace.path().to_str().unwrap();
    assert!(
        workspace
            .run(&["init", root, "--executor", "mock", "--json"])
            .status
            .success()
    );
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

    let run = workspace.run(&[
        "run",
        "--workspace",
        root,
        "--task",
        "must not start",
        "--json",
    ]);
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("lock and published snapshot differ"));
}
