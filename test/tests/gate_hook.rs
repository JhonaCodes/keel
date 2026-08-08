// SPDX-License-Identifier: Apache-2.0
//! Black-box contract of `keel gate` — the client-hook bridge. A Claude Code
//! PreToolUse payload is piped in; keel evaluates it and returns exit 2 to
//! block. This proves keel can govern the client's INTERNAL tool calls
//! (Write/Edit) that the wrapper cannot otherwise see — without needing a real
//! `claude`.

use keel_tests::hermetic::keel_bin;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("keel-gate-hook-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn path(&self) -> &Path {
        &self.root
    }
    fn run(&self, args: &[&str]) -> Output {
        let mut c = Command::new(keel_bin());
        c.env_clear();
        if let Ok(p) = std::env::var("PATH") {
            c.env("PATH", p);
        }
        c.current_dir(&self.root);
        c.args(args).output().expect("spawn keel")
    }
    /// Pipes `stdin` to `keel <args>` and returns the output (for `keel gate`).
    fn run_stdin(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(keel_bin())
            .env_clear()
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn keel gate");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().expect("wait keel gate")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A rule that forbids EDITING `.md` files (on file.edited), so a Claude
/// Write(notes.md) — an internal tool call — is blocked through the hook.
fn author_no_edit_md(root: &Path) {
    let rules = root.join("global/rules");
    fs::create_dir_all(&rules).unwrap();
    fs::write(
        rules.join("no-edit-md.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: global.no-edit-md, author: test, adrRef: adr:ADR-003, reviewAfter: P6M }
spec:
  on: [file.edited]
  scope: { paths: { include: ["**/*.md"] } }
  validate: { using: "builtin:text.regex", with: { pattern: "." } }
  enforcement:
    invalid: { decision: block, report: { message: "editing .md is forbidden here" } }
    valid: { decision: allow }
"#,
    )
    .unwrap();
}

/// The evidence-capture pair: a rule that records `test.completed` (Invalid
/// when the run FAILED) and a rule that forbids editing a prod `.rs` file until
/// such RED evidence exists this session.
fn author_require_red_via_captured_evidence(root: &Path) {
    let rules = root.join("global/rules");
    fs::create_dir_all(&rules).unwrap();
    fs::write(
        rules.join("record-test.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: global.record-test, author: test, adrRef: adr:ADR-004, reviewAfter: P6M }
spec:
  on: [test.completed]
  validate: { using: "builtin:text.contains", with: { value: "FAILED" } }
  enforcement:
    invalid: { decision: allow }
    valid: { decision: allow }
"#,
    )
    .unwrap();
    fs::write(
        rules.join("require-red.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: global.require-red, author: test, adrRef: adr:ADR-005, reviewAfter: P6M }
spec:
  on: [file.edited]
  scope: { paths: { include: ["**/*.rs"] } }
  preconditions:
    - using: "builtin:evidence.recorded"
      with: { event: "test.completed", verdict: invalid }
      onFail: block
  enforcement:
    valid: { decision: allow }
"#,
    )
    .unwrap();
}

/// evidence-capture end to end through the claude-code bridge: a real `cargo
/// test` completion that FAILED (PostToolUse Bash, non-zero exit) is turned
/// into durable `test.completed`/Invalid evidence by keel itself, which then
/// UNBLOCKS a `.rs` write the same session — no hand-fed native event. Before
/// the test run, the write is blocked; after it, allowed.
#[test]
fn a_failing_test_run_captured_by_keel_unblocks_a_write_the_same_session() {
    let ws = Workspace::new();
    let root = ws.path().to_str().unwrap().to_string();
    assert!(ws.run(&["init", &root, "--json"]).status.success());
    author_require_red_via_captured_evidence(ws.path());
    assert!(ws.run(&["compile", "--workspace", &root]).status.success());

    let gate_args = [
        "gate",
        "--client",
        "claude-code",
        "--workspace",
        &root,
        "--session",
        "s1",
    ];

    // No evidence yet → a .rs write is blocked.
    let write = r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Write","tool_input":{"file_path":"src/lib.rs","content":"fn f(){}"}}"#;
    let blocked = ws.run_stdin(&gate_args, write);
    assert_eq!(
        blocked.status.code(),
        Some(2),
        "a .rs write with no prior RED must be blocked: {}",
        String::from_utf8_lossy(&blocked.stderr)
    );

    // keel OBSERVES a failing `cargo test` (PostToolUse Bash, non-zero exit) and
    // records test.completed/Invalid itself — feedback-only, so exit 0.
    let post_test = r#"{"hook_event_name":"PostToolUse","session_id":"s1","tool_name":"Bash","tool_input":{"command":"cargo test --workspace"},"tool_response":{"exit_code":101,"stdout":"test result: FAILED. 1 failed"}}"#;
    let captured = ws.run_stdin(&gate_args, post_test);
    assert_eq!(
        captured.status.code(),
        Some(0),
        "capturing a completed test run is post-hoc, never a block"
    );

    // Same session, now WITH the captured RED evidence → the write is allowed.
    let allowed = ws.run_stdin(&gate_args, write);
    assert_eq!(
        allowed.status.code(),
        Some(0),
        "the .rs write must pass once keel captured the failing test: {}",
        String::from_utf8_lossy(&allowed.stderr)
    );
}

#[test]
fn the_hook_bridge_blocks_an_internal_write_via_pretooluse() {
    let ws = Workspace::new();
    let root = ws.path().to_str().unwrap().to_string();
    assert!(ws.run(&["init", &root, "--json"]).status.success());
    author_no_edit_md(ws.path());
    assert!(ws.run(&["compile", "--workspace", &root]).status.success());

    // A Claude Code PreToolUse(Write) payload for a .md file.
    let payload = r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Write","tool_input":{"file_path":"notes.md","content":"hi"}}"#;
    let out = ws.run_stdin(
        &[
            "gate",
            "--client",
            "claude-code",
            "--workspace",
            &root,
            "--session",
            "s1",
        ],
        payload,
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "PreToolUse Write of a .md must be blocked (exit 2): {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("BLOCKED (global.no-edit-md)"),
        "the packet must reach the model via stderr"
    );

    // A .txt Write is allowed (exit 0) — same tool, different target.
    let ok_payload = r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Write","tool_input":{"file_path":"notes.txt","content":"hi"}}"#;
    let ok = ws.run_stdin(
        &[
            "gate",
            "--client",
            "claude-code",
            "--workspace",
            &root,
            "--session",
            "s1",
        ],
        ok_payload,
    );
    assert_eq!(ok.status.code(), Some(0), "a .txt edit is allowed");

    // PostToolUse (post-hoc) never blocks even for a .md — the tool already ran
    // (invariant 8: no false exit-2).
    let post = r#"{"hook_event_name":"PostToolUse","session_id":"s1","tool_name":"Write","tool_input":{"file_path":"notes.md","content":"hi"}}"#;
    let post_out = ws.run_stdin(
        &[
            "gate",
            "--client",
            "claude-code",
            "--workspace",
            &root,
            "--session",
            "s1",
        ],
        post,
    );
    assert_eq!(
        post_out.status.code(),
        Some(0),
        "post-hoc feedback must not exit 2"
    );
}
