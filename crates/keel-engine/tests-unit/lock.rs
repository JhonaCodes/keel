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
    let a = Lock::generate(&binding(), &s, "0.1.0", BTreeMap::new());
    let b = Lock::generate(&binding(), &s, "0.1.0", BTreeMap::new());
    assert_eq!(a, b, "same binding + snapshot must yield an identical lock");
    assert!(!a.snapshot_hash.is_empty());
}

#[test]
fn verify_accepts_matching_snapshot() {
    let s = empty_snapshot();
    let lock = Lock::generate(&binding(), &s, "0.1.0", BTreeMap::new());
    assert!(lock.verify(&binding(), &s, "0.1.0").is_ok());
}

#[test]
fn verify_rejects_hash_drift() {
    let s = empty_snapshot();
    let mut lock = Lock::generate(&binding(), &s, "0.1.0", BTreeMap::new());
    lock.snapshot_hash = "sha256:deadbeef".into();
    let err = lock.verify(&binding(), &s, "0.1.0").unwrap_err();
    assert!(err.contains("hash drift"), "{err}");
}

#[test]
fn verify_rejects_binding_drift() {
    let s = empty_snapshot();
    let lock = Lock::generate(&binding(), &s, "0.1.0", BTreeMap::new());
    let other = ProjectBinding {
        project: "project:other/repo".into(),
        workspace: "org:local".into(),
    };
    let err = lock.verify(&other, &s, "0.1.0").unwrap_err();
    assert!(err.contains("binding drift"), "{err}");
}

/// The whole point of `knowledge_checkpoints`: it is persisted (round-trips
/// into the lock, so `keel knowledge verify` can later read it as an
/// external witness), but a session's chain growing between locks must NEVER
/// surface as `keel lock --verify` drift — that is a job left entirely to
/// `keel knowledge verify`, a separate integrity check.
#[test]
fn knowledge_checkpoints_are_persisted_but_never_compared_as_drift() {
    let s = empty_snapshot();
    let mut checkpoints = BTreeMap::new();
    checkpoints.insert("knowledge:jflow-memory".to_string(), "sha256:aaa".into());
    let lock = Lock::generate(&binding(), &s, "0.1.0", checkpoints.clone());
    assert_eq!(
        lock.knowledge_checkpoints, checkpoints,
        "checkpoints round-trip into the lock"
    );

    // Same snapshot, same binding, same version — the ONLY thing that could
    // differ from a freshly-generated lock is knowledge_checkpoints (real
    // value here vs. empty in a fresh regenerate). verify() must still pass.
    assert!(
        lock.verify(&binding(), &s, "0.1.0").is_ok(),
        "a lock carrying real knowledge_checkpoints must still verify clean \
         against the same snapshot — growth is not drift"
    );

    // Serialization round-trip: the field must survive write/read, since
    // `keel knowledge verify` depends on reading it back from disk later.
    let yaml = serde_yaml_ng::to_string(&lock).unwrap();
    let back: Lock = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(back.knowledge_checkpoints, checkpoints);
}

/// A lock with NO knowledge components declared serializes without the
/// `knowledgeCheckpoints` key at all (empty map is skipped) — existing
/// locks predating this field stay byte-for-byte unaffected.
#[test]
fn empty_knowledge_checkpoints_are_omitted_from_serialization() {
    let s = empty_snapshot();
    let lock = Lock::generate(&binding(), &s, "0.1.0", BTreeMap::new());
    let yaml = serde_yaml_ng::to_string(&lock).unwrap();
    assert!(
        !yaml.contains("knowledgeCheckpoints"),
        "empty checkpoints must not appear in the lock file: {yaml}"
    );
}
