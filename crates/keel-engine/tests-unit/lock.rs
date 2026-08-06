// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `lock` (relocated out of src; included via #[path] in src/lock.rs).

use super::*;
use std::collections::BTreeMap;

fn empty_snapshot() -> Snapshot {
    Snapshot::build(
        vec![],
        BTreeMap::new(),
        BTreeMap::new(),
        "2026-01-01T00:00:00Z".into(),
    )
    .unwrap()
}

fn binding() -> ProjectBinding {
    ProjectBinding {
        project: "project:acme/app".into(),
        workspace: "org:local".into(),
    }
}

#[test]
fn generate_is_deterministic() {
    let s = empty_snapshot();
    let a = Lock::generate(&binding(), &s, "0.1.0");
    let b = Lock::generate(&binding(), &s, "0.1.0");
    assert_eq!(a, b, "same binding + snapshot must yield an identical lock");
    assert!(!a.snapshot_hash.is_empty());
}

#[test]
fn verify_accepts_matching_snapshot() {
    let s = empty_snapshot();
    let lock = Lock::generate(&binding(), &s, "0.1.0");
    assert!(lock.verify(&binding(), &s, "0.1.0").is_ok());
}

#[test]
fn verify_rejects_hash_drift() {
    let s = empty_snapshot();
    let mut lock = Lock::generate(&binding(), &s, "0.1.0");
    lock.snapshot_hash = "sha256:deadbeef".into();
    let err = lock.verify(&binding(), &s, "0.1.0").unwrap_err();
    assert!(err.contains("hash drift"), "{err}");
}

#[test]
fn verify_rejects_binding_drift() {
    let s = empty_snapshot();
    let lock = Lock::generate(&binding(), &s, "0.1.0");
    let other = ProjectBinding {
        project: "project:other/repo".into(),
        workspace: "org:local".into(),
    };
    let err = lock.verify(&other, &s, "0.1.0").unwrap_err();
    assert!(err.contains("binding drift"), "{err}");
}
