// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `snapshot` (relocated out of src; included via #[path] in src/snapshot.rs).

use super::*;

fn rule(id: &str) -> CompiledRule {
    CompiledRule {
        id: id.into(),
        version: None,
        author: "t".into(),
        adr_ref: "adr:ADR-1".into(),
        review_after: "P6M".into(),
        reversibility: None,
        scope: None,
        on: vec![EventKind::FileEdited],
        when: None,
        detect: None,
        preconditions: vec![],
        validate: None,
        enforcement: CompiledEnforcement::default(),
        constraints: None,
    }
}

/// Invariant 9: same content → same hash, even if created_at changes.
#[test]
fn hash_ignores_created_at_and_is_stable() {
    let a = Snapshot::build(
        vec![rule("r1")],
        BTreeMap::new(),
        BTreeMap::new(),
        "2026-01-01T00:00:00Z".into(),
    )
    .unwrap();
    let b = Snapshot::build(
        vec![rule("r1")],
        BTreeMap::new(),
        BTreeMap::new(),
        "2026-06-01T12:00:00Z".into(),
    )
    .unwrap();
    assert_eq!(a.hash, b.hash);

    let c = Snapshot::build(
        vec![rule("r2")],
        BTreeMap::new(),
        BTreeMap::new(),
        "2026-01-01T00:00:00Z".into(),
    )
    .unwrap();
    assert_ne!(a.hash, c.hash);
}

/// A snapshot tampered with on disk does not load.
#[test]
fn tampered_snapshot_fails_to_load() {
    let dir = std::env::temp_dir().join(format!("keel-snap-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("snapshot.json");

    let snap = Snapshot::build(
        vec![rule("r1")],
        BTreeMap::new(),
        BTreeMap::new(),
        "2026-01-01T00:00:00Z".into(),
    )
    .unwrap();
    snap.save(&path).unwrap();

    // Alter the content without recomputing the hash.
    let tampered = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\"r1\"", "\"r1-tampered\"");
    std::fs::write(&path, tampered).unwrap();

    let err = Snapshot::load(&path).unwrap_err();
    assert!(matches!(err, SnapshotError::HashMismatch { .. }));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn scope_matches_paths_and_infers_language() {
    let scope = CompiledScope {
        languages: vec!["dart".into()],
        include: vec!["lib/**".into()],
        exclude: vec!["lib/generated/**".into()],
    };
    assert!(scope.matches(Some("lib/a.dart"), None));
    assert!(!scope.matches(Some("lib/generated/a.dart"), None));
    assert!(!scope.matches(Some("src/a.dart"), None));
    assert!(!scope.matches(Some("lib/a.php"), None));
    // Event without a file + scope with paths → no match.
    assert!(!scope.matches(None, Some("dart")));
}
