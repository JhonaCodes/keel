// SPDX-License-Identifier: Apache-2.0
//! Black-box contract of `keel mcp` over its real stdio transport: a toy MCP
//! client speaks newline-delimited JSON-RPC to the compiled binary and gets
//! its governed skill back — the convergence plane end to end.

use keel_tests::hermetic::keel_bin;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("keel-mcp-stdio-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn path(&self) -> &Path {
        &self.root
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        let mut c = Command::new(keel_bin());
        c.env_clear();
        if let Ok(p) = std::env::var("PATH") {
            c.env("PATH", p);
        }
        c.current_dir(&self.root);
        c.args(args).output().expect("spawn keel")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn author_skill(root: &Path) {
    let skills = root.join("global/skills");
    fs::create_dir_all(&skills).unwrap();
    fs::write(
        skills.join("access_keel.md"),
        "USE the query builder, not raw SQL.",
    )
    .unwrap();
    fs::write(
        skills.join("access-patterns.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Skill
metadata:
  id: access-patterns
  version: 0.1.0
spec:
  compact: skills/access_keel.md
"#,
    )
    .unwrap();
}

/// Same as `author_skill` but with a real `full` variant, for the on-demand
/// full-delivery test.
fn author_skill_with_full(root: &Path) {
    let skills = root.join("global/skills");
    fs::create_dir_all(&skills).unwrap();
    fs::write(
        skills.join("access_keel.md"),
        "USE the query builder, not raw SQL.",
    )
    .unwrap();
    fs::write(
        skills.join("access_full_keel.md"),
        "FULL: every query-builder method, with examples and edge cases.",
    )
    .unwrap();
    fs::write(
        skills.join("access-patterns.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Skill
metadata:
  id: access-patterns
  version: 0.1.0
spec:
  compact: skills/access_keel.md
  full: skills/access_full_keel.md
"#,
    )
    .unwrap();
}

#[test]
fn a_toy_client_lists_and_loads_a_governed_skill_over_stdio() {
    let ws = Workspace::new();
    let root = ws.path().to_str().unwrap().to_string();

    assert!(ws.run(&["init", &root, "--json"]).status.success());
    author_skill(ws.path());
    let compile = ws.run(&["compile", "--workspace", &root]);
    assert!(
        compile.status.success(),
        "compile: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    // Spawn the MCP server on stdio and drive it like a client would.
    let mut child = Command::new(keel_bin())
        .args(["mcp", "--workspace", &root, "--session", "session-mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn keel mcp");

    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    let rpc = |stdin: &mut dyn Write, reader: &mut dyn BufRead, req: Value| -> Value {
        serde_json::to_writer(&mut *stdin, &req).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };

    let init = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":0,"method":"initialize"}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "keel");

    let list = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
               "params":{"name":"keel.skills.list","arguments":{}}}),
    );
    let listing = list["result"]["content"][0]["text"].as_str().unwrap();
    assert!(listing.contains("access-patterns"), "listing: {listing}");

    let load = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
               "params":{"name":"keel.skills.load","arguments":{"id":"access-patterns"}}}),
    );
    let content = load["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        content.contains("USE the query builder"),
        "the governed skill content came back over stdio: {content}"
    );

    drop(stdin); // EOF → the server loop ends.
    let _ = child.wait();
}

/// A skill's `full` content is the authoritative context, so `keel.skills.load`
/// serves it by default. The compact variant remains available only as an
/// explicit preview (`full:false`).
#[test]
fn keel_skills_load_serves_full_content_by_default() {
    let ws = Workspace::new();
    let root = ws.path().to_str().unwrap().to_string();

    assert!(ws.run(&["init", &root, "--json"]).status.success());
    author_skill_with_full(ws.path());
    let compile = ws.run(&["compile", "--workspace", &root]);
    assert!(
        compile.status.success(),
        "compile: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let mut child = Command::new(keel_bin())
        .args(["mcp", "--workspace", &root, "--session", "session-full"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn keel mcp");

    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    let rpc = |stdin: &mut dyn Write, reader: &mut dyn BufRead, req: Value| -> Value {
        serde_json::to_writer(&mut *stdin, &req).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };

    let _init = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":0,"method":"initialize"}),
    );

    // Explicit full=false → compact preview.
    let compact = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
               "params":{"name":"keel.skills.load","arguments":{"id":"access-patterns","full":false}}}),
    );
    let compact_text = compact["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        compact_text.contains("(compact)") && compact_text.contains("USE the query builder"),
        "explicit full:false must serve the compact preview: {compact_text}"
    );

    // Default (full omitted) → full authoritative skill context.
    let full_default = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
               "params":{"name":"keel.skills.load","arguments":{"id":"access-patterns"}}}),
    );
    let full_default_text = full_default["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(
        full_default_text.contains("(full)")
            && full_default_text.contains("FULL: every query-builder method"),
        "default request must serve the full variant, not compact: {full_default_text}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// A governed agent whose executor is a LOCAL CLI: the "other model" is a
/// script that emits JSON. keel routes to it, validates the output against the
/// agent's outputSchema, and returns it — cross-model, no provider API.
fn author_agent(root: &Path) {
    // The executor "model": a CLI that ignores stdin and prints a fixed
    // verdict. Stands in for `codex exec` returning structured output.
    let bin = root.join("global/bin");
    fs::create_dir_all(&bin).unwrap();
    let script = bin.join("fake-auditor.sh");
    fs::write(
        &script,
        "#!/bin/sh\ncat >/dev/null\nprintf '{\"verdict\":\"ok\",\"note\":\"looks fine\"}'\n",
    )
    .unwrap();
    fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    let execs = root.join("global/executors");
    fs::create_dir_all(&execs).unwrap();
    fs::write(
        execs.join("auditor-cli.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: ModelExecutor
metadata: { id: auditor-cli, version: 1.0.0 }
spec:
  config:
    command: [sh, global/bin/fake-auditor.sh]
"#,
    )
    .unwrap();

    let agents = root.join("global/agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(
        agents.join("auditor.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Agent
metadata: { id: auditor }
spec:
  role: audit
  executor: executor:auditor-cli
  objective: Audit the diff for issues.
  outputSchema: global/agents/verdict.schema.json
"#,
    )
    .unwrap();
    fs::write(
        agents.join("verdict.schema.json"),
        r#"{ "type": "object", "required": ["verdict"], "properties": { "verdict": { "type": "string" }, "note": { "type": "string" } } }"#,
    )
    .unwrap();
}

#[test]
fn agent_invoke_routes_to_a_local_cli_executor_and_validates_its_output() {
    let ws = Workspace::new();
    let root = ws.path().to_str().unwrap().to_string();

    assert!(ws.run(&["init", &root, "--json"]).status.success());
    author_agent(ws.path());
    let compile = ws.run(&["compile", "--workspace", &root]);
    assert!(
        compile.status.success(),
        "compile: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let mut child = Command::new(keel_bin())
        .args(["mcp", "--workspace", &root, "--session", "session-agent"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn keel mcp");
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let rpc = |stdin: &mut dyn Write, reader: &mut dyn BufRead, req: Value| -> Value {
        serde_json::to_writer(&mut *stdin, &req).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };

    let resp = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
               "params":{"name":"keel.agent.invoke",
                         "arguments":{"agent":"auditor","input":"diff --git a b"}}}),
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("validated output") && text.contains("looks fine"),
        "the local-CLI agent's validated output came back: {text}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// A local-CLI auditor that returns a GO verdict, plus a record-audit rule on
/// `task.completed`: invoking the agent must record that verdict as evidence in
/// the session's ledger, so the audit gate can see it — the same evidence the
/// Task-tool hook records, now reachable via keel.agent.invoke.
fn author_go_auditor(root: &Path) {
    let bin = root.join("global/bin");
    fs::create_dir_all(&bin).unwrap();
    let script = bin.join("fake-go-auditor.sh");
    fs::write(
        &script,
        "#!/bin/sh\ncat >/dev/null\nprintf 'Audit report: no issues.\\n{\"verdict\":\"GO\",\"note\":\"ship it\"}'\n",
    )
    .unwrap();
    fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    let execs = root.join("global/executors");
    fs::create_dir_all(&execs).unwrap();
    fs::write(
        execs.join("go-auditor-cli.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: ModelExecutor
metadata: { id: go-auditor-cli, version: 1.0.0 }
spec:
  config:
    command: [sh, global/bin/fake-go-auditor.sh]
"#,
    )
    .unwrap();

    let agents = root.join("global/agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(
        agents.join("go-auditor.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Agent
metadata: { id: go-auditor }
spec:
  role: audit
  executor: executor:go-auditor-cli
  objective: Audit the diff for issues.
  outputSchema: global/agents/verdict.schema.json
"#,
    )
    .unwrap();
    fs::write(
        agents.join("verdict.schema.json"),
        r#"{ "type": "object", "required": ["verdict"], "properties": { "verdict": { "type": "string" }, "note": { "type": "string" } } }"#,
    )
    .unwrap();

    let rules = root.join("global/rules");
    fs::create_dir_all(&rules).unwrap();
    fs::write(
        rules.join("record-audit.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: record-audit, author: test, adrRef: adr:TEST-001, reviewAfter: P6M }
spec:
  on: [task.completed]
  validate: { using: "builtin:text.contains", with: { value: "VERDICT: GO" } }
  enforcement:
    invalid: { decision: allow }
    valid: { decision: allow }
"#,
    )
    .unwrap();
}

#[test]
fn agent_invoke_records_a_task_completed_verdict_for_the_gate() {
    let ws = Workspace::new();
    let root = ws.path().to_str().unwrap().to_string();

    assert!(ws.run(&["init", &root, "--json"]).status.success());
    author_go_auditor(ws.path());
    let compile = ws.run(&["compile", "--workspace", &root]);
    assert!(
        compile.status.success(),
        "compile: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let mut child = Command::new(keel_bin())
        .args(["mcp", "--workspace", &root, "--session", "session-agent"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn keel mcp");
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let rpc = |stdin: &mut dyn Write, reader: &mut dyn BufRead, req: Value| -> Value {
        serde_json::to_writer(&mut *stdin, &req).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };

    let resp = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
               "params":{"name":"keel.agent.invoke",
                         "arguments":{"agent":"go-auditor","input":"diff --git a b"}}}),
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("VERDICT: GO"),
        "invoke reflects the recorded verdict: {text}"
    );

    drop(stdin);
    let _ = child.wait();

    // The evidence landed in THIS session's ledger as record-audit invalid (=GO),
    // exactly the row the commit gate keys on.
    let files = keel_engine::workspace::WorkspaceFiles::empty(ws.path().to_path_buf());
    let ledger = keel_engine::ledger::Ledger::open(&files.ledger_path()).unwrap();
    let evidence = ledger.recorded_evidence("session-agent").unwrap();
    assert!(
        evidence.iter().any(|(kind, verdict)| matches!(
            kind,
            keel_core::event::EventKind::TaskCompleted
        ) && matches!(verdict, keel_core::Verdict::Invalid)),
        "task.completed GO evidence must be recorded: {evidence:?}"
    );
}
