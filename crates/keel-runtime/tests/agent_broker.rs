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
            match_: Default::default(),
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
            match_: Default::default(),
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

#[test]
fn output_schema_is_injected_into_the_prompt_and_json_is_extracted_from_prose() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("verdict.schema.json"),
        r#"{"type":"object","required":["verdict"],"properties":{"verdict":{"type":"string"}}}"#,
    )
    .unwrap();
    let agents = BTreeMap::from([(
        "auditor".to_string(),
        CompiledAgent {
            id: "auditor".to_string(),
            role: "audit".to_string(),
            executor: "claude".to_string(),
            objective: Some("Audit the change".to_string()),
            match_: Default::default(),
            output_schema: Some("verdict.schema.json".to_string()),
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
    // A realistic auditor reply: a prose report (that itself contains braces)
    // ending with the verdict JSON on its own line.
    let mut executor = MockModelExecutor::with_response(ModelResponse::text(
        "Audit report\nsome pseudo-code { not json }\nfinal call below:\n{\"verdict\": \"GO\", \"note\": \"ok\"}",
    ));

    let result = broker
        .invoke(
            "session",
            "auditor",
            "inspect",
            &mut scheduler,
            &mut executor,
        )
        .unwrap();

    // The verdict JSON is extracted from the trailing object, not the prose braces.
    let output = result
        .output
        .expect("schema agent must yield validated output");
    assert_eq!(output.get("verdict").and_then(|v| v.as_str()), Some("GO"));
    // The schema was injected into the prompt the executor received.
    let prompt = &executor.requests()[0].messages[0].content;
    assert!(prompt.contains("verdict"), "prompt should carry the schema");
    assert!(
        prompt.contains("JSON"),
        "prompt should instruct JSON-only output"
    );
}

#[test]
fn agent_without_output_schema_gets_no_injection() {
    let root = tempfile::tempdir().unwrap();
    let agents = BTreeMap::from([(
        "reviewer".to_string(),
        CompiledAgent {
            id: "reviewer".to_string(),
            role: "review".to_string(),
            executor: "codex".to_string(),
            objective: Some("Review architecture".to_string()),
            match_: Default::default(),
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
    let mut executor = MockModelExecutor::with_response(ModelResponse::text("looks good"));

    let result = broker
        .invoke(
            "session",
            "reviewer",
            "inspect",
            &mut scheduler,
            &mut executor,
        )
        .unwrap();

    assert!(result.output.is_none());
    assert_eq!(result.content, "looks good");
    let prompt = &executor.requests()[0].messages[0].content;
    assert_eq!(prompt, "Objective: Review architecture\n\nInput:\ninspect");
}
