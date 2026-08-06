// SPDX-License-Identifier: Apache-2.0
//! Integration test for adapter preflight (spec section 12.1, invariant 8) via the
//! real `keel` binary: `keel adapter claude-code --check` passes for a block on
//! an event the client can prevent (`command.requested`) and fails (exit 1) for
//! a block on one it cannot (`transition.requested`).

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
    fn new(on_event: &str) -> Ws {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("keel-adapter-it-{}-{n}", std::process::id()));
        fs::create_dir_all(root.join("rules")).unwrap();
        fs::write(
            root.join("workspace.yaml"),
            "apiVersion: keel/v1alpha1\nkind: Workspace\nmetadata: { id: it-adapter }\nspec: {}\n",
        )
        .unwrap();
        let rule = format!(
            "apiVersion: keel/v1alpha1\nkind: Rule\n\
             metadata: {{ id: it.block, version: 1.0.0, author: t, adrRef: adr:ADR-1, reviewAfter: P6M }}\n\
             spec:\n  reversibility: irreversible\n  on: [{on_event}]\n\
             \x20 detect:   {{ using: builtin:text.contains, with: {{ value: \"X\" }} }}\n\
             \x20 validate: {{ using: builtin:text.contains, with: {{ value: \"X\" }} }}\n\
             \x20 enforcement:\n    invalid: {{ decision: block }}\n    unknown: {{ decision: deny-pending-approval }}\n    valid: {{ decision: allow }}\n"
        );
        fs::write(root.join("rules").join("r.yaml"), rule).unwrap();
        Ws { root }
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

/// A block on `command.requested` is honorable by claude-code — preflight OK.
#[test]
fn preflight_passes_for_blockable_event() {
    let ws = Ws::new("command.requested");
    assert_eq!(ws.run(&["compile"]), 0);
    assert_eq!(
        ws.run(&["adapter", "claude-code", "--check"]),
        0,
        "a block on command.requested is enforceable by claude-code"
    );
}

/// A block on `transition.requested` is a false promise — claude-code has no
/// hook to prevent it — so preflight must fail (exit 1).
#[test]
fn preflight_fails_for_unblockable_event() {
    let ws = Ws::new("transition.requested");
    assert_eq!(ws.run(&["compile"]), 0);
    assert_eq!(
        ws.run(&["adapter", "claude-code", "--check"]),
        1,
        "a block on transition.requested cannot be honored by claude-code"
    );
}
