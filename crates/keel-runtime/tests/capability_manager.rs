use keel_runtime::{CapabilityError, CapabilityManager, CapabilityRequest};

#[test]
fn denied_capability_has_no_side_effect_and_granted_write_stays_in_workspace() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = CapabilityManager::new(root.path());
    let request = CapabilityRequest::new(
        "filesystem.write",
        serde_json::json!({"path": "out/result.txt", "content": "governed"}),
    );

    assert!(matches!(
        manager.execute(&request),
        Err(CapabilityError::NotGranted { .. })
    ));
    assert!(!root.path().join("out/result.txt").exists());

    manager.grant("filesystem.write");
    manager.execute(&request).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("out/result.txt")).unwrap(),
        "governed"
    );

    let escape = CapabilityRequest::new(
        "filesystem.write",
        serde_json::json!({"path": "../escape.txt", "content": "bad"}),
    );
    assert!(matches!(
        manager.execute(&escape),
        Err(CapabilityError::OutsideWorkspace { .. })
    ));
    assert!(!root.path().parent().unwrap().join("escape.txt").exists());
}

#[cfg(unix)]
#[test]
fn write_rejects_a_symlink_that_points_outside_the_workspace() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.path().join("linked")).unwrap();
    let mut manager = CapabilityManager::new(root.path());
    manager.grant("filesystem.write");

    let result = manager.execute(&CapabilityRequest::new(
        "filesystem.write",
        serde_json::json!({"path": "linked/escape.txt", "content": "bad"}),
    ));
    assert!(matches!(
        result,
        Err(CapabilityError::OutsideWorkspace { .. })
    ));
    assert!(!outside.path().join("escape.txt").exists());
}
