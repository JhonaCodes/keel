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

/// The verify-before-close pair (H-009): a rule that records `task.completed`
/// (Invalid when the subagent's text says NO-GO) and a rule that blocks Stop
/// unless a GO (Valid) audit was recorded this session.
fn author_require_go_before_close(root: &Path) {
    let rules = root.join("global/rules");
    fs::create_dir_all(&rules).unwrap();
    fs::write(
        rules.join("record-task.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: global.record-task, author: test, adrRef: adr:ADR-006, reviewAfter: P6M }
spec:
  on: [task.completed]
  validate: { using: "builtin:text.contains", with: { value: "NO-GO" } }
  enforcement:
    invalid: { decision: allow }
    valid: { decision: allow }
"#,
    )
    .unwrap();
    fs::write(
        rules.join("require-go-before-close.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: global.require-go-before-close, author: test, adrRef: adr:ADR-007, reviewAfter: P6M }
spec:
  on: [completion.requested]
  preconditions:
    - using: "builtin:evidence.recorded"
      with: { event: "task.completed", verdict: valid }
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

/// Prompt enrichment (D-013): a `prompt.submitted` rule whose tool emits a
/// finding has that finding DELIVERED to the model as `additionalContext` —
/// deterministically, by code, on the operator's prompt. The model receives the
/// task already deserialized; it chose nothing.
#[test]
fn a_prompt_submitted_rule_injects_its_tool_output_as_additional_context() {
    let ws = Workspace::new();
    let root = ws.path().to_str().unwrap().to_string();
    assert!(ws.run(&["init", &root, "--json"]).status.success());

    // A tool that, seeing a linear.app URL in the prompt, returns the decomposed
    // ticket as a finding (here: a fixed marker so the test is hermetic).
    let tools = ws.path().join("global/tools");
    fs::create_dir_all(&tools).unwrap();
    fs::write(
        tools.join("enrich.py"),
        r#"import sys, json
event = json.load(sys.stdin)
content = (event.get("content") or "")
if "linear.app" in content:
    print(json.dumps({"verdict": "valid", "findings": [{"message": "TICKET ABC-123: unify form labels"}]}))
else:
    print(json.dumps({"verdict": "valid", "findings": []}))
"#,
    )
    .unwrap();
    let rules = ws.path().join("global/rules");
    fs::create_dir_all(&rules).unwrap();
    fs::write(
        rules.join("enrich.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: global.enrich-linear, author: test, adrRef: adr:ADR-013, reviewAfter: P6M }
spec:
  on: [prompt.submitted]
  validate: { using: "tool:enrich" }
  enforcement:
    valid: { decision: allow }
"#,
    )
    .unwrap();
    fs::write(
        ws.path().join("global/tools/enrich.tool.yaml"),
        "apiVersion: keel/v1alpha1\nkind: Tool\nmetadata: { id: enrich, version: 0.1.0 }\nspec:\n  command: [python3, global/tools/enrich.py]\n  output: verdict-json\n",
    )
    .unwrap();
    assert!(ws.run(&["compile", "--workspace", &root]).status.success());

    let payload = r#"{"hook_event_name":"UserPromptSubmit","session_id":"s1","prompt":"explain https://linear.app/acme/issue/ABC-123/x"}"#;
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
        Some(0),
        "enrichment never blocks a prompt"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("additionalContext") && stdout.contains("TICKET ABC-123"),
        "the tool output must be delivered to the model as additionalContext: {stdout}"
    );
    // The operator gets a visible confirmation of WHAT was delivered.
    assert!(
        stdout.contains("systemMessage") && stdout.contains("global.enrich-linear"),
        "a systemMessage must name the contributing rule for the operator: {stdout}"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("keel ✦ contexto entregado"),
        "the stderr banner is the fallback visible channel"
    );

    // A prompt WITHOUT a linear URL yields no injected context (no finding).
    let plain =
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"s1","prompt":"just say hi"}"#;
    let out2 = ws.run_stdin(
        &[
            "gate",
            "--client",
            "claude-code",
            "--workspace",
            &root,
            "--session",
            "s1",
        ],
        plain,
    );
    assert_eq!(out2.status.code(), Some(0));
    assert!(
        !String::from_utf8_lossy(&out2.stdout).contains("additionalContext"),
        "nothing to inject → no context output"
    );
}

/// Opportune delivery (D-016): editing a Dart file is a MOMENT — keel surfaces
/// the relevant governed skill as `additionalContext` on the PreToolUse edit,
/// WITHOUT blocking (exit 0, no permissionDecision). "You're about to touch a
/// widget → keel has these."
#[test]
fn a_file_edit_surfaces_the_relevant_skill_without_blocking() {
    let ws = Workspace::new();
    let root = ws.path().to_str().unwrap().to_string();
    assert!(ws.run(&["init", &root, "--json"]).status.success());

    let skills = ws.path().join("global/skills");
    fs::create_dir_all(&skills).unwrap();
    fs::write(
        skills.join("keel_flutter_widgets_keel.md"),
        "Use widget CLASSES, never widget-returning functions.",
    )
    .unwrap();
    fs::write(
        skills.join("keel_flutter_widgets.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Skill
metadata: { id: keel_flutter_widgets, version: 0.1.0 }
spec:
  description: Flutter widget class rules
  match: { terms: [widget] }
  compact: global/skills/keel_flutter_widgets_keel.md
"#,
    )
    .unwrap();
    assert!(ws.run(&["compile", "--workspace", &root]).status.success());

    // A PreToolUse Edit of a .dart file whose content mentions a widget.
    let payload = r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Edit","tool_input":{"file_path":"lib/home.dart","new_string":"class Home extends StatelessWidget { Widget build(c) => Container(); }"}}"#;
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
        Some(0),
        "opportune delivery never blocks the edit"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("additionalContext") && stdout.contains("keel_flutter_widgets"),
        "the relevant skill must be surfaced at the edit moment: {stdout}"
    );
    assert!(
        stdout.contains("PreToolUse") && !stdout.contains("permissionDecision"),
        "delivery uses the PreToolUse channel and never overrides the permission flow: {stdout}"
    );

    // Editing an unrelated file (no matching term) surfaces nothing — no noise.
    let plain = r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Edit","tool_input":{"file_path":"README.txt","new_string":"hello there"}}"#;
    let out2 = ws.run_stdin(
        &[
            "gate",
            "--client",
            "claude-code",
            "--workspace",
            &root,
            "--session",
            "s1",
        ],
        plain,
    );
    assert_eq!(out2.status.code(), Some(0));
    assert!(
        !String::from_utf8_lossy(&out2.stdout).contains("additionalContext"),
        "a trivial edit surfaces nothing"
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

/// H-009 end to end through the claude-code bridge: a real `Task` subagent
/// completion (a code-auditor/edu-revisor style GO/NO-GO reviewer) is turned
/// into durable `task.completed` evidence by keel itself — no hand-fed native
/// event — which then gates `Stop`: blocked with no audit at all, still
/// blocked after a NO-GO verdict, allowed only after a GO verdict.
#[test]
fn a_completed_task_subagent_captured_by_keel_gates_stop_on_its_go_no_go_verdict() {
    let ws = Workspace::new();
    let root = ws.path().to_str().unwrap().to_string();
    assert!(ws.run(&["init", &root, "--json"]).status.success());
    author_require_go_before_close(ws.path());
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

    // No audit at all yet → Stop is blocked.
    let stop = r#"{"hook_event_name":"Stop","session_id":"s1"}"#;
    let blocked = ws.run_stdin(&gate_args, stop);
    assert_eq!(
        blocked.status.code(),
        Some(2),
        "Stop with no recorded audit must be blocked: {}",
        String::from_utf8_lossy(&blocked.stderr)
    );

    // keel OBSERVES a completed code-auditor Task subagent saying NO-GO
    // (PostToolUse, feedback-only, exit 0) and records task.completed/Invalid
    // itself.
    let post_task_nogo = r#"{"hook_event_name":"PostToolUse","session_id":"s1","tool_name":"Task","tool_input":{"subagent_type":"code-auditor"},"tool_response":{"content":[{"type":"text","text":"Veredicto: NO-GO - missing coverage on the auth path"}]}}"#;
    let captured = ws.run_stdin(&gate_args, post_task_nogo);
    assert_eq!(
        captured.status.code(),
        Some(0),
        "capturing a completed Task run is post-hoc, never a block"
    );

    // Still blocked: the only recorded audit evidence is NO-GO, not GO.
    let still_blocked = ws.run_stdin(&gate_args, stop);
    assert_eq!(
        still_blocked.status.code(),
        Some(2),
        "a NO-GO audit must not unblock Stop"
    );

    // The auditor re-runs and now says GO → keel records task.completed/Valid.
    let post_task_go = r#"{"hook_event_name":"PostToolUse","session_id":"s1","tool_name":"Task","tool_input":{"subagent_type":"code-auditor"},"tool_response":{"content":[{"type":"text","text":"Veredicto: GO - all checks pass"}]}}"#;
    let captured2 = ws.run_stdin(&gate_args, post_task_go);
    assert_eq!(captured2.status.code(), Some(0));

    // Same session, now with GO evidence → Stop is allowed.
    let allowed = ws.run_stdin(&gate_args, stop);
    assert_eq!(
        allowed.status.code(),
        Some(0),
        "a GO audit must unblock Stop: {}",
        String::from_utf8_lossy(&allowed.stderr)
    );
}
