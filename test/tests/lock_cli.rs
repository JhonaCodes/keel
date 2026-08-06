// SPDX-License-Identifier: Apache-2.0
//! Integration test for the binding + lock ceremony (spec section 8.6, invariants 4
//! & 9) via the real `keel` binary: bind → compile → lock → verify, and drift
//! detection (a changed snapshot makes `keel lock --verify` fail with exit 1).

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
        let root = std::env::temp_dir().join(format!("keel-lock-it-{}-{n}", std::process::id()));
        fs::create_dir_all(root.join("rules")).unwrap();
        fs::write(
            root.join("workspace.yaml"),
            "apiVersion: keel/v1alpha1\nkind: Workspace\nmetadata: { id: it-lock }\nspec: {}\n",
        )
        .unwrap();
        Ws { root }
    }

    fn write_rule(&self, needle: &str) {
        let rule = format!(
            "apiVersion: keel/v1alpha1\nkind: Rule\n\
             metadata: {{ id: it.r, version: 1.0.0, author: t, adrRef: adr:ADR-1, reviewAfter: P6M }}\n\
             spec:\n  reversibility: reversible\n  on: [file.edited]\n\
             \x20 detect:   {{ using: builtin:text.contains, with: {{ value: \"{needle}\" }} }}\n\
             \x20 validate: {{ using: builtin:text.contains, with: {{ value: \"{needle}\" }} }}\n\
             \x20 enforcement:\n    invalid: {{ decision: block }}\n    valid: {{ decision: allow }}\n"
        );
        fs::write(self.root.join("rules").join("r.yaml"), rule).unwrap();
    }

    fn run(&self, args: &[&str]) -> i32 {
        let out = Command::new(keel_bin())
            .args(args)
            .arg("--workspace")
            .arg(&self.root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("spawn keel");
        out.status.code().unwrap_or(-1)
    }
}

impl Drop for Ws {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn bind_lock_verify_and_drift() {
    let ws = Ws::new();
    ws.write_rule("TODO");

    // bind writes .keel/project.yaml
    assert_eq!(
        ws.run(&["bind", "--project", "project:test/app"]),
        0,
        "bind should succeed"
    );
    assert!(ws.root.join(".keel/project.yaml").exists());

    // compile then lock
    assert_eq!(ws.run(&["compile"]), 0, "compile should succeed");
    assert_eq!(ws.run(&["lock"]), 0, "lock should succeed");
    assert!(ws.root.join(".keel/keel.lock").exists());

    // verify passes against the same snapshot
    assert_eq!(ws.run(&["lock", "--verify"]), 0, "verify should pass");

    // change a rule → recompile → the snapshot hash drifts → verify fails
    ws.write_rule("FIXME");
    assert_eq!(ws.run(&["compile"]), 0, "recompile should succeed");
    assert_eq!(
        ws.run(&["lock", "--verify"]),
        1,
        "verify must fail (exit 1) when the snapshot drifted from the lock"
    );

    // regenerating the lock reconciles it
    assert_eq!(ws.run(&["lock"]), 0, "re-lock should succeed");
    assert_eq!(
        ws.run(&["lock", "--verify"]),
        0,
        "verify should pass again after re-locking"
    );
}
