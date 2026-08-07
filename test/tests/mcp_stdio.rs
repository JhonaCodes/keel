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
        skills.join("access.md"),
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
  compact: global/skills/access.md
"#,
    )
    .unwrap();
}

#[test]
fn a_toy_client_lists_and_loads_a_governed_skill_over_stdio() {
    let ws = Workspace::new();
    let root = ws.path().to_str().unwrap().to_string();

    assert!(
        ws.run(&["init", &root, "--executor", "mock", "--json"])
            .status
            .success()
    );
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
