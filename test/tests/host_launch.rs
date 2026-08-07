// SPDX-License-Identifier: Apache-2.0
//! Black-box contract of the parent runtime: `keel launch` contains a child
//! process and a governed command inside it is decided BEFORE it exists.
//!
//! This is the owner's canonical acceptance shape: a global rule forbids
//! deleting `.md` files (external bash validator, exit-code contract) while
//! `.txt` deletions pass — exercised through the real binary, real shims,
//! real broker socket and a real `/bin/sh` child under the PTY.

use keel_tests::hermetic::keel_bin;
use rusqlite::Connection;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("keel-host-launch-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // The dir must exist up front: `run` uses it as cwd (the launch
        // resolves relative targets like `notes.md` against it).
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(keel_bin());
        command.env_clear();
        if let Ok(path) = std::env::var("PATH") {
            command.env("PATH", path);
        }
        command.current_dir(&self.root);
        command.args(args).output().expect("spawn keel")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// The owner's rule, verbatim in shape: a global Rule whose verdict comes
/// from an external bash tool — exit 1 (block) for `rm` of a `.md` path,
/// exit 0 (allow) otherwise.
fn author_no_delete_md(root: &Path) {
    let tools = root.join("global/tools");
    let rules = root.join("global/rules");
    fs::create_dir_all(&tools).unwrap();
    fs::create_dir_all(&rules).unwrap();

    let script = tools.join("no-delete-md.sh");
    fs::write(
        &script,
        "#!/bin/sh\n\
         payload=\"$(cat)\"\n\
         cmd=\"$(printf '%s' \"$payload\" | sed -n 's/.*\"command\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p')\"\n\
         first=\"$(printf '%s' \"$cmd\" | awk '{print $1}')\"\n\
         case \"${first##*/}\" in\n\
           rm|unlink)\n\
             if printf '%s' \"$cmd\" | grep -qiE '\\.md($|[^a-zA-Z0-9])'; then exit 1; fi ;;\n\
         esac\n\
         exit 0\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    fs::write(
        tools.join("no-delete-md.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Tool
metadata:
  id: no-delete-md
  version: 0.1.0
spec:
  command: [sh, global/tools/no-delete-md.sh]
  timeoutMs: 5000
  output: exit-code
"#,
    )
    .unwrap();

    fs::write(
        rules.join("no-delete-md.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Rule
metadata:
  id: global.no-delete-md
  author: test
  adrRef: adr:ADR-001
  reviewAfter: P6M
spec:
  reversibility: irreversible
  on: [command.requested]
  validate:
    using: tool:no-delete-md
  enforcement:
    invalid:
      decision: block
      report:
        message: "deleting .md files is forbidden"
    valid:
      decision: allow
"#,
    )
    .unwrap();
}

fn blocked_evidence_count(root: &Path) -> i64 {
    let ledger = Connection::open(root.join(".keel-state/ledger.sqlite")).unwrap();
    ledger
        .query_row(
            "SELECT COUNT(*) FROM evidence WHERE rule_id = 'global.no-delete-md' \
             AND effective_decision = '\"block\"'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn a_governed_rm_is_decided_before_it_exists_as_a_process() {
    let workspace = Workspace::new();
    let root = workspace.path().to_str().unwrap().to_string();

    let init = workspace.run(&["init", &root, "--executor", "mock", "--json"]);
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    author_no_delete_md(workspace.path());
    let compile = workspace.run(&["compile", "--workspace", &root]);
    assert!(
        compile.status.success(),
        "compile failed: {}\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr)
    );

    fs::write(workspace.path().join("notes.md"), "keep me\n").unwrap();
    fs::write(workspace.path().join("notes.txt"), "expendable\n").unwrap();

    // Blocked: the .md deletion never runs — the file survives, the child's
    // exit code is the shim's 2, and the packet reached the child's terminal.
    let blocked = workspace.run(&[
        "launch",
        "--client",
        "generic",
        "--workspace",
        &root,
        "--",
        "/bin/sh",
        "-c",
        "rm notes.md",
    ]);
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&blocked.stdout),
        String::from_utf8_lossy(&blocked.stderr)
    );
    assert!(
        !blocked.status.success(),
        "the governed rm must fail: {transcript}"
    );
    assert_eq!(
        blocked.status.code(),
        Some(2),
        "exit contract: {transcript}"
    );
    assert!(
        workspace.path().join("notes.md").exists(),
        "a blocked command never exists as a process"
    );
    assert!(
        transcript.contains("BLOCKED (global.no-delete-md)"),
        "the packet must reach the transcript: {transcript}"
    );
    assert_eq!(blocked_evidence_count(workspace.path()), 1);

    // Allowed: the .txt deletion is the same command family, different target.
    let allowed = workspace.run(&[
        "launch",
        "--client",
        "generic",
        "--workspace",
        &root,
        "--",
        "/bin/sh",
        "-c",
        "rm notes.txt",
    ]);
    assert!(
        allowed.status.success(),
        "rm notes.txt must pass: {}{}",
        String::from_utf8_lossy(&allowed.stdout),
        String::from_utf8_lossy(&allowed.stderr)
    );
    assert!(
        !workspace.path().join("notes.txt").exists(),
        "the allowed command actually ran"
    );
    // Still exactly one block in evidence: the allow left its own entry with
    // a different decision, never a second block.
    assert_eq!(blocked_evidence_count(workspace.path()), 1);
}

#[test]
fn an_absolute_path_bypasses_shims_and_the_preflight_says_so_honestly() {
    // F1 scope honesty (invariant 8): PATH interposition governs the PATH
    // surface only. `/bin/rm` bypasses it — that is the OS-sandbox plane's
    // job (F2). This test PINS the limitation so F2 flips it consciously.
    let workspace = Workspace::new();
    let root = workspace.path().to_str().unwrap().to_string();

    let init = workspace.run(&["init", &root, "--executor", "mock", "--json"]);
    assert!(init.status.success());
    author_no_delete_md(workspace.path());
    assert!(
        workspace
            .run(&["compile", "--workspace", &root])
            .status
            .success()
    );

    fs::write(workspace.path().join("notes.md"), "keep me\n").unwrap();
    let bypass = workspace.run(&[
        "launch",
        "--client",
        "generic",
        "--workspace",
        &root,
        "--",
        "/bin/sh",
        "-c",
        "/bin/rm notes.md",
    ]);
    assert!(
        bypass.status.success(),
        "F1 documents this gap; F2 closes it: {}",
        String::from_utf8_lossy(&bypass.stderr)
    );
    assert!(
        !workspace.path().join("notes.md").exists(),
        "if this starts failing, F2 landed — move the assertion there"
    );
}
