// SPDX-License-Identifier: Apache-2.0
//! Integration tests for the `keel gate` EXIT-CODE CONTRACT (spec §5.3, §12.3).
//!
//! The exit code IS the contract with the client: 2 prevents the action, 0
//! allows it. `gate` reads the event from stdin, so the contract is exercised
//! here end-to-end by running the real `keel` binary against a self-contained
//! temp workspace that uses ONLY builtin detectors/validators (`text.contains`)
//! — no external tools, no Python, no network. Portable on CI.
//!
//! Run under `cargo test --workspace`, which builds the `keel` binary first.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Absolute path to the compiled `keel` binary. This test crate lives at
/// `keel/test/`, so the workspace target dir is `../target/<profile>/keel`.
/// `cargo test --workspace` builds the binary before running the tests.
fn keel_bin() -> PathBuf {
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

/// A throwaway workspace under the OS temp dir. Removed on drop.
struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Creates `workspace.yaml` plus one rule file and returns the workspace.
    fn new(rule_yaml: &str) -> Workspace {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("keel-gate-it-{}-{n}", std::process::id()));
        fs::create_dir_all(root.join("rules")).unwrap();
        fs::write(
            root.join("workspace.yaml"),
            "apiVersion: keel/v1alpha1\nkind: Workspace\nmetadata: { id: it-gate }\nspec: {}\n",
        )
        .unwrap();
        fs::write(root.join("rules").join("rule.yaml"), rule_yaml).unwrap();
        Workspace { root }
    }

    fn compile(&self) {
        let out = keel(&["compile", "--workspace", self.root_str()], None);
        assert!(
            out.status.success(),
            "compile failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Runs `keel gate` with `event_json` on stdin; returns the exit code.
    fn gate(&self, session: &str, event_json: &str) -> i32 {
        let out = keel(
            &["gate", "--workspace", self.root_str(), "--session", session],
            Some(event_json),
        );
        out.status.code().expect("gate returned no exit code")
    }

    fn root_str(&self) -> &str {
        self.root.to_str().unwrap()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Runs the compiled `keel` binary with the given args and optional stdin.
fn keel(args: &[&str], stdin: Option<&str>) -> std::process::Output {
    let bin = keel_bin();
    assert!(
        bin.exists(),
        "keel binary not found at {} — run via `cargo test --workspace`",
        bin.display()
    );
    let mut cmd = Command::new(&bin);
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

/// A rule that blocks an irreversible `command.requested` whose content
/// contains `DROP` (builtin detect + builtin validate; the violation pattern
/// is the content). Irreversible + inner-ring ⇒ a block is preventable.
const COMMAND_BLOCK_RULE: &str = r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: it.block-drop, version: 1.0.0, author: test, adrRef: adr:ADR-001, reviewAfter: P6M }
spec:
  reversibility: irreversible
  on: [command.requested]
  detect:   { using: builtin:text.contains, with: { value: "DROP" } }
  validate: { using: builtin:text.contains, with: { value: "DROP" } }
  enforcement:
    invalid: { decision: block, report: { message: "no DROP allowed" } }
    unknown: { decision: deny-pending-approval }
    valid:   { decision: allow }
"#;

/// A rule that blocks a reversible `file.edited` whose content contains
/// `BADPATTERN`. Outer ring ⇒ the block is FEEDBACK (exit 0) but it records a
/// live blocker used by the completion gate.
const FILE_BLOCK_RULE: &str = r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: it.block-bad, version: 1.0.0, author: test, adrRef: adr:ADR-002, reviewAfter: P6M }
spec:
  reversibility: reversible
  on: [file.edited]
  detect:   { using: builtin:text.contains, with: { value: "BADPATTERN" } }
  validate: { using: builtin:text.contains, with: { value: "BADPATTERN" } }
  enforcement:
    invalid: { decision: block, report: { message: "remove BADPATTERN" } }
    unknown: { decision: review }
    valid:   { decision: allow }
"#;

/// §5.3 inner ring: a violating pre-action command is blocked BEFORE it runs —
/// exit code 2. This is the assertion STATUS.md marked as only-indirect.
#[test]
fn violating_command_request_exits_2() {
    let ws = Workspace::new(COMMAND_BLOCK_RULE);
    ws.compile();
    let code = ws.gate(
        "s1",
        r#"{"kind":"command.requested","command":"psql -c stmt","content":"DROP DATABASE prod","session_id":"s1"}"#,
    );
    assert_eq!(
        code, 2,
        "a violating inner-ring command must exit 2 (blocked)"
    );
}

/// The same gate lets a clean command through with exit 0 — the contract's
/// other half (no false positives).
#[test]
fn clean_command_request_exits_0() {
    let ws = Workspace::new(COMMAND_BLOCK_RULE);
    ws.compile();
    let code = ws.gate(
        "s1",
        r#"{"kind":"command.requested","command":"psql -c stmt","content":"SELECT 1","session_id":"s1"}"#,
    );
    assert_eq!(code, 0, "a clean command must be allowed (exit 0)");
}

/// §5.3 outer ring: a post-hoc file edit that violates is FEEDBACK, never
/// prevention — exit 0 even though the finding is a block (the file already
/// landed; its danger only materializes at execution).
#[test]
fn violating_file_edit_is_feedback_exit_0() {
    let ws = Workspace::new(FILE_BLOCK_RULE);
    ws.compile();
    let code = ws.gate(
        "s2",
        r#"{"kind":"file.edited","file":"lib/a.txt","content":"BADPATTERN here","session_id":"s2"}"#,
    );
    assert_eq!(
        code, 0,
        "an outer-ring edit block is feedback (exit 0), not prevention"
    );
}

/// §12.3 completion gate: "done" is a transition the runtime authorizes. A
/// session with a live blocker (an invalid file finding never cleared) cannot
/// close — the completion request is denied with exit 2.
#[test]
fn completion_with_live_blocker_exits_2() {
    let ws = Workspace::new(FILE_BLOCK_RULE);
    ws.compile();
    // 1) An edit that violates records a live blocker (feedback, exit 0).
    let edit = ws.gate(
        "s3",
        r#"{"kind":"file.edited","file":"lib/a.txt","content":"BADPATTERN here","session_id":"s3"}"#,
    );
    assert_eq!(edit, 0, "the edit itself is feedback (exit 0)");

    // 2) Trying to close the session with that blocker alive is denied.
    let done = ws.gate("s3", r#"{"kind":"completion.requested","session_id":"s3"}"#);
    assert_eq!(
        done, 2,
        "completion with a live blocker must be denied (exit 2)"
    );
}

/// A clean session closes: no blockers ⇒ completion allowed (exit 0).
#[test]
fn completion_without_blockers_exits_0() {
    let ws = Workspace::new(FILE_BLOCK_RULE);
    ws.compile();
    let done = ws.gate("s4", r#"{"kind":"completion.requested","session_id":"s4"}"#);
    assert_eq!(done, 0, "a clean session must be allowed to close (exit 0)");
}
