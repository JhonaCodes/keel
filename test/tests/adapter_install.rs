// SPDX-License-Identifier: Apache-2.0
//! `keel adapter claude-code --install/--uninstall` (the installation story,
//! STATUS section 9): keel wires its own hook INTO the client's settings.json.
//! The logic lives in keel (portable to any machine), NOT in a hand-pasted
//! snippet. These tests prove it is MERGE-SAFE (preserves every other key and
//! the operator's own hooks), IDEMPOTENT (no duplicate on re-install), and that
//! UNINSTALL removes only keel's blocks.

use keel_tests::hermetic::keel_bin;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A workspace with a PRE-EXISTING settings.json carrying operator config
/// (model, a secret, own hooks) — the thing an install must never clobber.
fn ws_with_settings() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("keel-adapter-it-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join(".claude")).unwrap();
    fs::write(
        root.join("workspace.yaml"),
        "apiVersion: keel/v1alpha1\nkind: Workspace\nmetadata: { id: it }\nspec: {}\n",
    )
    .unwrap();
    fs::write(
        root.join(".claude/settings.json"),
        r#"{
  "model": "haiku",
  "secretThing": "keep-me",
  "hooks": {
    "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "bash $HOME/mine.sh" } ] } ]
  }
}
"#,
    )
    .unwrap();
    root
}

fn keel(args: &[&str]) -> bool {
    Command::new(keel_bin())
        .args(args)
        .output()
        .expect("spawn keel")
        .status
        .success()
}

fn settings(root: &Path) -> serde_json::Value {
    let raw = fs::read_to_string(root.join(".claude/settings.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn count_keel_blocks(v: &serde_json::Value, event: &str) -> usize {
    v["hooks"][event]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|b| {
                    b["hooks"]
                        .as_array()
                        .map(|hs| {
                            hs.iter().any(|h| {
                                h["command"]
                                    .as_str()
                                    .map(|c| c.contains("gate --client claude-code"))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

fn has_operator_hook(v: &serde_json::Value) -> bool {
    v["hooks"]["PreToolUse"]
        .as_array()
        .map(|a| {
            a.iter().any(|b| {
                b["hooks"]
                    .as_array()
                    .map(|hs| {
                        hs.iter()
                            .any(|h| h["command"].as_str() == Some("bash $HOME/mine.sh"))
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Install merges keel's hooks WITHOUT clobbering the operator's config or
/// hooks, and writes a backup.
#[test]
fn install_is_merge_safe() {
    let root = ws_with_settings();
    assert!(keel(&[
        "adapter",
        "claude-code",
        "--install",
        "--workspace",
        root.to_str().unwrap()
    ]));
    let v = settings(&root);

    assert_eq!(v["model"], "haiku", "operator config preserved");
    assert_eq!(v["secretThing"], "keep-me", "unrelated keys preserved");
    assert!(has_operator_hook(&v), "operator's own hook preserved");
    assert_eq!(
        count_keel_blocks(&v, "PreToolUse"),
        1,
        "keel PreToolUse added"
    );
    assert_eq!(
        count_keel_blocks(&v, "PostToolUse"),
        1,
        "keel PostToolUse added"
    );
    assert_eq!(count_keel_blocks(&v, "Stop"), 1, "keel Stop added");
    assert!(
        root.join(".claude/settings.json.keel-bak").exists(),
        "a backup is written before modifying"
    );
    let _ = fs::remove_dir_all(&root);
}

/// Installing twice does not duplicate keel's blocks.
#[test]
fn install_is_idempotent() {
    let root = ws_with_settings();
    let arg = root.to_str().unwrap();
    assert!(keel(&[
        "adapter",
        "claude-code",
        "--install",
        "--workspace",
        arg
    ]));
    assert!(keel(&[
        "adapter",
        "claude-code",
        "--install",
        "--workspace",
        arg
    ]));
    assert_eq!(
        count_keel_blocks(&settings(&root), "PreToolUse"),
        1,
        "re-install must not duplicate"
    );
    let _ = fs::remove_dir_all(&root);
}

/// Uninstall removes only keel's blocks; the operator's hook and config stay.
#[test]
fn uninstall_removes_only_keel() {
    let root = ws_with_settings();
    let arg = root.to_str().unwrap();
    assert!(keel(&[
        "adapter",
        "claude-code",
        "--install",
        "--workspace",
        arg
    ]));
    assert!(keel(&[
        "adapter",
        "claude-code",
        "--uninstall",
        "--workspace",
        arg
    ]));
    let v = settings(&root);
    assert_eq!(
        count_keel_blocks(&v, "PreToolUse"),
        0,
        "keel blocks removed"
    );
    assert!(has_operator_hook(&v), "operator hook untouched");
    assert_eq!(v["model"], "haiku", "config untouched");
    let _ = fs::remove_dir_all(&root);
}
