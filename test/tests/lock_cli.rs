// SPDX-License-Identifier: Apache-2.0
//! Integration test for the binding + lock ceremony (spec section 8.6, invariants 4
//! & 9) via the real `keel` binary: bind → compile → lock → verify, and drift
//! detection (a changed snapshot makes `keel lock --verify` fail with exit 1).

use rusqlite::Connection;
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

    fn write_knowledge(&self, id: &str, chain_path: &str) {
        fs::create_dir_all(self.root.join("knowledge")).unwrap();
        let doc = format!(
            "apiVersion: keel/v1alpha1\nkind: Knowledge\n\
             metadata: {{ id: {id}, version: 0.1.0 }}\n\
             spec:\n  content: {chain_path}\n"
        );
        fs::write(self.root.join("knowledge").join(format!("{id}.yaml")), doc).unwrap();
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

/// The Piece B guarantee, end to end through the real binary: a `Knowledge`
/// chain growing between locks is never `keel lock --verify` drift, the
/// checkpoint round-trips through `keel.lock` (readable back after a fresh
/// process invocation, not just in-memory), and `keel knowledge verify`
/// catches direct tampering on the chain's sqlite file that bypassed the
/// `keel knowledge append` API entirely.
#[test]
fn knowledge_growth_never_drifts_the_lock_and_verify_catches_tampering() {
    let ws = Ws::new();
    ws.write_rule("TODO");
    ws.write_knowledge("mem", ".keel-state/knowledge/mem.sqlite");
    assert_eq!(ws.run(&["bind", "--project", "project:test/app"]), 0);
    assert_eq!(ws.run(&["compile"]), 0);

    // Lock BEFORE anything has grown — no checkpoint yet, and that must not
    // be an error (a freshly locked workspace with untouched knowledge is
    // the common case, not a broken one).
    assert_eq!(ws.run(&["lock"]), 0);
    assert_eq!(ws.run(&["lock", "--verify"]), 0);

    // Grow the chain AFTER the lock — this must never surface as drift.
    assert_eq!(
        ws.run(&["knowledge", "append", "--id", "mem", "--content", "first"]),
        0
    );
    assert_eq!(
        ws.run(&["knowledge", "append", "--id", "mem", "--content", "second"]),
        0
    );
    assert_eq!(
        ws.run(&["lock", "--verify"]),
        0,
        "growth of a knowledge chain must never be reported as lock drift"
    );

    // Re-locking now DOES capture a real checkpoint — round-trip it through
    // a fresh process (not just in-memory) via `keel.lock` on disk.
    assert_eq!(ws.run(&["lock"]), 0);
    let lock_yaml = fs::read_to_string(ws.root.join(".keel/keel.lock")).unwrap();
    assert!(
        lock_yaml.contains("knowledge_checkpoints") && lock_yaml.contains("knowledge:mem"),
        "the checkpoint must be persisted in keel.lock: {lock_yaml}"
    );

    // More growth after THIS lock too — still not drift.
    assert_eq!(
        ws.run(&["knowledge", "append", "--id", "mem", "--content", "third"]),
        0
    );
    assert_eq!(ws.run(&["lock", "--verify"]), 0);

    // Sane baseline: verify reports an intact chain.
    assert_eq!(ws.run(&["knowledge", "verify"]), 0);

    // Tamper OUTSIDE the API entirely — direct SQL on the chain's own file —
    // simulating exactly what `evidence.recorded`'s design note warned about:
    // a rewrite that never went through `keel knowledge append`.
    let db = ws.root.join(".keel-state/knowledge/mem.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE knowledge_entries SET content = 'FORGED' WHERE seq = 0",
        [],
    )
    .unwrap();

    assert_eq!(
        ws.run(&["knowledge", "verify"]),
        1,
        "tampering with an old entry must be caught (exit 1)"
    );
}
