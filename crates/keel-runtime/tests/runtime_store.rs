use keel_engine::snapshot::{CompiledSkill, Snapshot};
use keel_runtime::{
    Operation, RuntimeError, RuntimeHost, RuntimeStore, SkillDefinition, SkillReadRequest,
    StoreError,
};
use std::collections::BTreeMap;

#[test]
fn persisted_receipt_restores_required_skill_consumption() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("runtime.sqlite");

    let store = RuntimeStore::open(&db).unwrap();
    let mut first = RuntimeHost::with_store("session-1", "sha256:one", store).unwrap();
    first.register_skill(SkillDefinition::new(
        "architecture.review",
        "1.0.0",
        "Read before planning.",
        None,
    ));
    first.require_skill("architecture.review");
    first
        .read_skill(SkillReadRequest::compact(
            "architecture.review",
            "investigation",
        ))
        .unwrap();
    drop(first);

    let store = RuntimeStore::open(&db).unwrap();
    let mut resumed = RuntimeHost::with_store("session-1", "sha256:one", store).unwrap();
    resumed.register_skill(SkillDefinition::new(
        "architecture.review",
        "1.0.0",
        "Read before planning.",
        None,
    ));
    resumed.require_skill("architecture.review");

    assert!(matches!(
        resumed.authorize(Operation::PlanSubmit),
        Err(RuntimeError::WrongPhase { .. })
    ));
}

#[test]
fn resuming_a_session_with_another_snapshot_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("runtime.sqlite");
    RuntimeHost::with_store("session-1", "sha256:one", RuntimeStore::open(&db).unwrap()).unwrap();

    let result =
        RuntimeHost::with_store("session-1", "sha256:two", RuntimeStore::open(&db).unwrap());

    assert!(matches!(
        result,
        Err(RuntimeError::Store {
            source: StoreError::SnapshotMismatch { .. }
        })
    ));
}

#[test]
fn durable_host_hydrates_skills_from_the_compiled_snapshot() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("compact.md"), "Keel guidance").unwrap();
    let mut skills = BTreeMap::new();
    skills.insert(
        "skill-a".to_string(),
        CompiledSkill {
            id: "skill-a".to_string(),
            description: None,
            version: "1.0.0".to_string(),
            compact: "compact.md".to_string(),
            full: None,
            examples: Vec::new(),
        },
    );
    let snapshot = Snapshot::build(Vec::new(), BTreeMap::new(), skills, "now".into()).unwrap();
    let store = RuntimeStore::open(&workspace.path().join("runtime.sqlite")).unwrap();

    let mut host = RuntimeHost::from_snapshot_with_store(
        "session-snapshot",
        &snapshot,
        workspace.path(),
        store,
    )
    .unwrap();
    let receipt = host
        .read_skill(SkillReadRequest::compact("skill-a", "investigation"))
        .unwrap();

    assert_eq!(receipt.content, "Keel guidance");
}

#[test]
fn session_execution_metadata_is_pinned_for_resume() {
    let store = RuntimeStore::open_in_memory().unwrap();
    store.ensure_session("session-1", "sha256:one").unwrap();
    store
        .ensure_session_metadata("session-1", "Review architecture", "claude")
        .unwrap();

    let metadata = store.session_metadata("session-1").unwrap().unwrap();
    assert_eq!(metadata.task, "Review architecture");
    assert_eq!(metadata.executor_id, "claude");
    assert!(matches!(
        store.ensure_session_metadata("session-1", "Different task", "codex"),
        Err(StoreError::MetadataMismatch { .. })
    ));
}
