use keel_engine::snapshot::{CompiledAgent, Snapshot};
use keel_runtime::{AgentBroker, AgentScheduler, MockModelExecutor, ModelResponse};
use std::collections::BTreeMap;

#[test]
fn logical_agent_routes_to_declared_cross_provider_executor() {
    let root = tempfile::tempdir().unwrap();
    let agents = BTreeMap::from([(
        "architecture-reviewer".to_string(),
        CompiledAgent {
            id: "architecture-reviewer".to_string(),
            role: "review".to_string(),
            executor: "codex".to_string(),
            objective: Some("Review architecture".to_string()),
            output_schema: None,
            timeout_ms: Some(10_000),
            max_tokens: Some(1_000),
        },
    )]);
    let snapshot = Snapshot::build_full(
        Vec::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        agents,
        "now".to_string(),
    )
    .unwrap();
    let broker = AgentBroker::from_snapshot(&snapshot, root.path());
    let mut scheduler = AgentScheduler::in_memory(1).unwrap();
    let mut executor = MockModelExecutor::with_response(ModelResponse::text("approved"));

    let result = broker
        .invoke(
            "session-parent-claude",
            "architecture-reviewer",
            "inspect",
            &mut scheduler,
            &mut executor,
        )
        .unwrap();

    assert_eq!(broker.executor_for("architecture-reviewer"), Some("codex"));
    assert_eq!(result.executor_id, "codex");
    assert_eq!(result.content, "approved");
}

#[test]
fn failed_agent_execution_releases_the_scheduler_slot() {
    let root = tempfile::tempdir().unwrap();
    let agents = BTreeMap::from([(
        "reviewer".to_string(),
        CompiledAgent {
            id: "reviewer".to_string(),
            role: "review".to_string(),
            executor: "codex".to_string(),
            objective: None,
            output_schema: None,
            timeout_ms: None,
            max_tokens: None,
        },
    )]);
    let snapshot = Snapshot::build_full(
        Vec::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        agents,
        "now".to_string(),
    )
    .unwrap();
    let broker = AgentBroker::from_snapshot(&snapshot, root.path());
    let mut scheduler = AgentScheduler::in_memory(1).unwrap();
    let mut empty_executor = MockModelExecutor::default();

    assert!(
        broker
            .invoke(
                "session",
                "reviewer",
                "inspect",
                &mut scheduler,
                &mut empty_executor,
            )
            .is_err()
    );
    let next = scheduler.submit("session", "reviewer", "codex").unwrap();
    assert_eq!(scheduler.claim_task(&next.id).unwrap().unwrap().id, next.id);
}
