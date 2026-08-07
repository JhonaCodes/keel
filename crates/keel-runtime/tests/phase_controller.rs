use keel_runtime::{
    ArtifactKind, Operation, Phase, PhaseError, RuntimeError, RuntimeHost, RuntimeStore,
    SkillDefinition, SkillReadRequest,
};
use serde_json::json;

fn investigation_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "required": ["problem"],
        "properties": { "problem": { "type": "string" } },
        "additionalProperties": false
    })
}

#[test]
fn phase_transitions_require_valid_artifacts_and_cannot_skip() {
    let mut host = RuntimeHost::new("session-phase", "sha256:one");
    assert_eq!(host.current_phase(), Phase::Investigation);

    let missing = host.advance_phase(Phase::Planning).unwrap_err();
    assert!(matches!(
        missing,
        RuntimeError::Phase {
            source: PhaseError::RequiredArtifactMissing { .. }
        }
    ));

    host.record_artifact(
        ArtifactKind::InvestigationReport,
        &json!({ "problem": "runtime ownership" }),
        &investigation_schema(),
    )
    .unwrap();
    host.advance_phase(Phase::Planning).unwrap();
    assert_eq!(host.current_phase(), Phase::Planning);
    assert!(host.authorize(Operation::PlanSubmit).is_ok());
    assert!(matches!(
        host.authorize(Operation::ActionRequest),
        Err(RuntimeError::WrongPhase { .. })
    ));

    let skipped = host.advance_phase(Phase::Audit).unwrap_err();
    assert!(matches!(
        skipped,
        RuntimeError::Phase {
            source: PhaseError::InvalidTransition { .. }
        }
    ));
}

#[test]
fn invalid_artifact_does_not_unlock_a_transition() {
    let mut host = RuntimeHost::new("session-invalid", "sha256:one");
    host.record_artifact(
        ArtifactKind::InvestigationReport,
        &json!({}),
        &investigation_schema(),
    )
    .unwrap();

    assert!(matches!(
        host.advance_phase(Phase::Planning),
        Err(RuntimeError::Phase {
            source: PhaseError::RequiredArtifactMissing { .. }
        })
    ));
}

#[test]
fn phase_and_artifacts_are_restored_after_process_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("runtime.sqlite");
    let mut first = RuntimeHost::with_store(
        "session-phase",
        "sha256:one",
        RuntimeStore::open(&db).unwrap(),
    )
    .unwrap();
    first
        .record_artifact(
            ArtifactKind::InvestigationReport,
            &json!({ "problem": "runtime ownership" }),
            &investigation_schema(),
        )
        .unwrap();
    first.advance_phase(Phase::Planning).unwrap();
    drop(first);

    let resumed = RuntimeHost::with_store(
        "session-phase",
        "sha256:one",
        RuntimeStore::open(&db).unwrap(),
    )
    .unwrap();
    assert_eq!(resumed.current_phase(), Phase::Planning);
    assert!(resumed.authorize(Operation::PlanSubmit).is_ok());
}

#[test]
fn model_cannot_forge_the_phase_recorded_by_skill_read() {
    let mut host = RuntimeHost::new("session-forged-phase", "sha256:one");
    host.register_skill(SkillDefinition::new("skill-a", "1.0.0", "content", None));

    assert!(matches!(
        host.read_skill(SkillReadRequest::compact("skill-a", "planning")),
        Err(RuntimeError::RequestPhaseMismatch { .. })
    ));
}

#[test]
fn restored_transition_rejects_an_invalid_guard_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("runtime.sqlite");
    let mut host = RuntimeHost::with_store(
        "session-corrupt",
        "sha256:one",
        RuntimeStore::open(&db).unwrap(),
    )
    .unwrap();
    let invalid = host
        .record_artifact(
            ArtifactKind::InvestigationReport,
            &json!({}),
            &investigation_schema(),
        )
        .unwrap();
    drop(host);

    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute(
            "INSERT INTO phase_transitions
             (transition_id, session_id, from_phase, to_phase, guard_artifact_id, advanced_at)
             VALUES ('forged', 'session-corrupt', 'investigation', 'planning', ?1, ?2)",
            rusqlite::params![invalid.artifact_id, jiff::Timestamp::now().to_string()],
        )
        .unwrap();
    drop(connection);

    let result = RuntimeHost::with_store(
        "session-corrupt",
        "sha256:one",
        RuntimeStore::open(&db).unwrap(),
    );
    assert!(matches!(
        result,
        Err(RuntimeError::Phase {
            source: PhaseError::InvalidHistory { .. }
        })
    ));
}

#[test]
fn malformed_artifact_schema_is_rejected_instead_of_treated_as_a_failed_artifact() {
    let mut host = RuntimeHost::new("session-bad-schema", "sha256:one");

    assert!(matches!(
        host.record_artifact(
            ArtifactKind::InvestigationReport,
            &json!({ "problem": "x" }),
            &json!({ "type": 42 }),
        ),
        Err(RuntimeError::ArtifactSchema { .. })
    ));
}

#[test]
fn future_phase_artifact_cannot_be_preloaded() {
    let mut host = RuntimeHost::new("session-future-artifact", "sha256:one");

    assert!(matches!(
        host.record_artifact(
            ArtifactKind::SolutionContract,
            &json!({ "problem": "x" }),
            &investigation_schema(),
        ),
        Err(RuntimeError::Phase {
            source: PhaseError::ArtifactNotAllowed { .. }
        })
    ));
}
