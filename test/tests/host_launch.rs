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

/// The OS-sandbox backstop: a global Containment forbidding deletion of any
/// `.md` file, regardless of how `rm` is invoked.
fn author_containment_no_md(root: &Path) {
    let dir = root.join("global/containment");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("no-md.yaml"),
        r#"apiVersion: keel/v1alpha1
kind: Containment
metadata:
  id: global.hard.no-md
spec:
  denyUnlink: ["**/*.md"]
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

    let init = workspace.run(&["init", &root, "--json"]);
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

/// F2: the OS-sandbox backstop closes the absolute-path bypass that shims
/// alone cannot. macOS-only for now — the Linux provider is a later phase
/// (F2b); until it lands, Linux degrades to shims and this guarantee does not
/// hold there (documented, not silent).
#[cfg(target_os = "macos")]
#[test]
fn the_os_sandbox_blocks_an_absolute_path_bypass() {
    let workspace = Workspace::new();
    let root = workspace.path().to_str().unwrap().to_string();

    let init = workspace.run(&["init", &root, "--json"]);
    assert!(init.status.success());
    author_containment_no_md(workspace.path());
    let compile = workspace.run(&["compile", "--workspace", &root]);
    assert!(
        compile.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    fs::write(workspace.path().join("notes.md"), "keep me\n").unwrap();
    // /bin/rm sidesteps PATH interposition entirely — only the kernel can stop
    // it. With containment + Seatbelt, it must fail and the file must survive.
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
        !bypass.status.success(),
        "the OS sandbox must refuse /bin/rm of a .md: {}{}",
        String::from_utf8_lossy(&bypass.stdout),
        String::from_utf8_lossy(&bypass.stderr)
    );
    assert!(
        workspace.path().join("notes.md").exists(),
        "the kernel refused the unlink — the .md survives even via absolute path"
    );
}

/// The honest downgrade: `--containment shims` opts out of the hard ring, so
/// the absolute-path bypass works again — and the banner said so. This pins
/// that shims-only is a real, announced level, not an accident.
#[test]
fn shims_only_mode_leaves_the_absolute_path_bypass_open_and_says_so() {
    let workspace = Workspace::new();
    let root = workspace.path().to_str().unwrap().to_string();

    let init = workspace.run(&["init", &root, "--json"]);
    assert!(init.status.success());
    author_containment_no_md(workspace.path());
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
        "--containment",
        "shims",
        "--",
        "/bin/sh",
        "-c",
        "/bin/rm notes.md",
    ]);
    let transcript = String::from_utf8_lossy(&bypass.stderr).to_string();
    assert!(
        transcript.contains("shims-only") || transcript.contains("containment: shims"),
        "the downgrade must be announced: {transcript}"
    );
    assert!(
        !workspace.path().join("notes.md").exists(),
        "shims-only lets the absolute-path bypass through (by explicit request)"
    );
}

/// The supervisor (P3) surfaces a suggestion to the OPERATOR when the model
/// oscillates — the same rule blocking three times in a session. It never
/// writes into the model's stream. `--no-suggest` silences it.
#[test]
fn the_supervisor_surfaces_an_oscillation_and_no_suggest_silences_it() {
    let workspace = Workspace::new();
    let root = workspace.path().to_str().unwrap().to_string();
    assert!(workspace.run(&["init", &root, "--json"]).status.success());
    author_no_delete_md(workspace.path());
    assert!(
        workspace
            .run(&["compile", "--workspace", &root])
            .status
            .success()
    );

    // Three governed `.md` deletions in one session = an oscillation. The
    // trailing sleep gives the ~750ms supervisor poll a chance to fire before
    // teardown (deterministic: 1.5s > one poll interval).
    let script = "rm a.md; rm b.md; rm c.md; sleep 1.5";
    let with = workspace.run(&[
        "launch",
        "--client",
        "generic",
        "--workspace",
        &root,
        "--",
        "/bin/sh",
        "-c",
        script,
    ]);
    let transcript = String::from_utf8_lossy(&with.stderr);
    assert!(
        transcript.contains("[keel] suggestion") && transcript.contains("global.no-delete-md"),
        "the supervisor must surface the oscillation: {transcript}"
    );

    let silenced = workspace.run(&[
        "launch",
        "--client",
        "generic",
        "--workspace",
        &root,
        "--no-suggest",
        "--",
        "/bin/sh",
        "-c",
        script,
    ]);
    assert!(
        !String::from_utf8_lossy(&silenced.stderr).contains("[keel] suggestion"),
        "--no-suggest must silence the supervisor"
    );
}
