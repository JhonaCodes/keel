// SPDX-License-Identifier: Apache-2.0

use keel_tests::hermetic::keel_bin;
use rusqlite::Connection;
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

#[test]
fn workflow_requirement_is_consumed_through_keel_before_the_session_advances() {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("keel-components-it-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let root_arg = root.to_str().unwrap();

    assert!(keel(&["init", root_arg, "--executor", "mock", "--json"]).success());
    fs::create_dir_all(root.join("projects/app/skills")).unwrap();
    fs::create_dir_all(root.join("projects/app/workflows")).unwrap();
    fs::write(
        root.join("projects/app/skills/architecture.md"),
        "Read before planning.",
    )
    .unwrap();
    fs::write(
        root.join("projects/app/skills/architecture.yaml"),
        "apiVersion: keel/v1alpha1\nkind: Skill\nmetadata: { id: architecture, version: 1.0.0 }\nspec: { compact: projects/app/skills/architecture.md }\n",
    )
    .unwrap();
    fs::write(
        root.join("projects/app/workflows/default.yaml"),
        "apiVersion: keel/v1alpha1\nkind: Workflow\nmetadata: { id: default, version: 1.0.0 }\nspec:\n  requirements:\n    - { component: 'skill:architecture', phases: [investigation], required: true }\n",
    )
    .unwrap();

    assert!(keel(&["compile", "--workspace", root_arg]).success());
    assert!(keel(&["lock", "--workspace", root_arg]).success());
    let run = keel(&[
        "run",
        "--workspace",
        root_arg,
        "--task",
        "Review architecture",
        "--executor",
        "mock",
        "--json",
    ]);
    assert!(run.success(), "run must consume required components");

    let connection = Connection::open(root.join(".keel-state/runtime.sqlite")).unwrap();
    let count: u64 = connection
        .query_row(
            "SELECT COUNT(*) FROM component_receipts WHERE component_kind = 'skill' AND component_id = 'architecture'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    let _ = fs::remove_dir_all(root);
}

fn keel(args: &[&str]) -> std::process::ExitStatus {
    Command::new(keel_bin())
        .env_clear()
        .args(args)
        .output()
        .expect("spawn keel")
        .status
}
