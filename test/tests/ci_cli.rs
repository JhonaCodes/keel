// SPDX-License-Identifier: Apache-2.0
//! Integration test for the compliance plane `keel ci` (spec section 5.2, section 8) via the
//! real `keel` binary: `resolve` gates on binding + compile + lock, and fails
//! (non-zero) on a missing binding, a missing lock, or lock drift.

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
        let root = std::env::temp_dir().join(format!("keel-ci-it-{}-{n}", std::process::id()));
        fs::create_dir_all(root.join("rules")).unwrap();
        fs::write(
            root.join("workspace.yaml"),
            "apiVersion: keel/v1alpha1\nkind: Workspace\nmetadata: { id: it-ci }\nspec: {}\n",
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

/// An unbound repo does not resolve — `ci resolve` fails before doing work.
#[test]
fn ci_resolve_fails_without_binding() {
    let ws = Ws::new();
    ws.write_rule("TODO");
    assert_ne!(
        ws.run(&["ci", "resolve"]),
        0,
        "resolve must fail when there is no binding"
    );
}

/// A bound + locked repo resolves; then a rule change (without re-locking)
/// makes `ci resolve` fail on drift; re-locking reconciles it.
#[test]
fn ci_resolve_gates_on_lock_drift() {
    let ws = Ws::new();
    ws.write_rule("TODO");

    assert_eq!(ws.run(&["bind", "--project", "project:test/app"]), 0);
    assert_eq!(ws.run(&["compile"]), 0);
    assert_eq!(ws.run(&["lock"]), 0);

    // Everything resolves.
    assert_eq!(
        ws.run(&["ci", "resolve"]),
        0,
        "resolve should pass when locked"
    );
    assert_eq!(ws.run(&["ci", "run"]), 0, "run should pass when resolved");

    // Change a rule but do NOT re-lock: the fresh compile drifts from the lock.
    ws.write_rule("FIXME");
    assert_ne!(
        ws.run(&["ci", "resolve"]),
        0,
        "resolve must fail on lock drift"
    );

    // Re-locking reconciles the resolution.
    assert_eq!(ws.run(&["lock"]), 0);
    assert_eq!(
        ws.run(&["ci", "resolve"]),
        0,
        "resolve should pass again after re-locking"
    );
}
