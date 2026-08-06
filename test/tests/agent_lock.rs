// SPDX-License-Identifier: Apache-2.0
//! Integration test for invariant 14: agents/executors are part of the locked
//! configuration. Changing an executor's `model` must be detectable drift —
//! `keel lock --verify` fails (exit 1) — because the model is hashed into the
//! snapshot and pinned by the lock.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn keel_bin() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.push("target");
    p.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    p.push("keel");
    p
}

struct Ws {
    root: PathBuf,
}

impl Ws {
    fn new() -> Ws {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("keel-agentlock-it-{}-{n}", std::process::id()));
        fs::create_dir_all(root.join("agents")).unwrap();
        fs::write(
            root.join("workspace.yaml"),
            "apiVersion: keel/v1alpha1\nkind: Workspace\nmetadata: { id: it-agentlock }\nspec: {}\n",
        )
        .unwrap();
        Ws { root }
    }

    /// Writes an Agent + its AgentExecutor with the given executor model.
    fn write_agent(&self, model: &str) {
        let doc = format!(
            "apiVersion: keel/v1alpha1\nkind: Agent\n\
             metadata: {{ id: rev, version: 1.0.0 }}\n\
             spec: {{ role: audit, executor: executor:local }}\n\
             ---\n\
             apiVersion: keel/v1alpha1\nkind: AgentExecutor\n\
             metadata: {{ id: local, version: 0.1.0 }}\n\
             spec: {{ command: [echo, \"{{}}\"], model: {model} }}\n"
        );
        fs::write(self.root.join("agents").join("reviewer.yaml"), doc).unwrap();
    }

    fn run(&self, args: &[&str]) -> i32 {
        Command::new(keel_bin())
            .args(args)
            .arg("--workspace")
            .arg(&self.root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("spawn keel")
            .status
            .code()
            .unwrap_or(-1)
    }
}

impl Drop for Ws {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn executor_model_change_is_lock_drift() {
    let ws = Ws::new();
    ws.write_agent("model-a");

    assert_eq!(ws.run(&["bind", "--project", "project:test/app"]), 0);
    assert_eq!(ws.run(&["compile"]), 0, "compile with an agent+executor");
    assert_eq!(ws.run(&["lock"]), 0);
    assert_eq!(
        ws.run(&["lock", "--verify"]),
        0,
        "freshly locked → verify ok"
    );

    // Change ONLY the executor's model, without re-locking.
    ws.write_agent("model-b");
    assert_eq!(ws.run(&["compile"]), 0, "recompile with the new model");
    assert_eq!(
        ws.run(&["lock", "--verify"]),
        1,
        "a changed executor model must be detected as lock drift (invariant 14)"
    );

    // Re-locking reconciles it.
    assert_eq!(ws.run(&["lock"]), 0);
    assert_eq!(ws.run(&["lock", "--verify"]), 0, "verify ok after re-lock");
}
