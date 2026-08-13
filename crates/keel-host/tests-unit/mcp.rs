// SPDX-License-Identifier: Apache-2.0
//! Unit tests for the MCP server dispatch (relocated; included via #[path]).
//!
//! These drive `Server::handle` directly with JSON-RPC values — the stdio
//! framing is a thin newline loop tested end-to-end in `test/tests/`.

use super::*;
use keel_engine::snapshot::CompiledSkill;
use std::collections::BTreeMap;

fn server_with_skill() -> (Server, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("skills")).unwrap();
    std::fs::write(root.join("skills/access.md"), "USE the query builder.").unwrap();

    let mut skills = BTreeMap::new();
    skills.insert(
        "access-patterns".to_string(),
        CompiledSkill {
            id: "access-patterns".into(),
            description: None,
            match_: Default::default(),
            version: "0.1.0".into(),
            compact: "skills/access.md".into(),
            full: None,
            examples: vec![],
        },
    );
    let snapshot = Snapshot::build(vec![], BTreeMap::new(), skills, "t".into()).unwrap();
    let files = WorkspaceFiles::empty(root.to_path_buf());
    std::fs::create_dir_all(files.state_dir()).unwrap();
    let store = RuntimeStore::open(&files.state_dir().join(RUNTIME_DB)).unwrap();
    store
        .ensure_session("session-test", &snapshot.hash.to_string())
        .unwrap();
    let server = Server {
        root: root.to_path_buf(),
        snapshot,
        sessions: SessionStore::new(&files.state_dir()),
        store,
        session_id: "session-test".into(),
    };
    (server, dir)
}

fn call(server: &mut Server, tool: &str, args: Value) -> String {
    let req = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": tool, "arguments": args }
    });
    let resp = server.handle(&req).expect("a request gets a response");
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn initialize_advertises_tools_capability() {
    let (mut server, _dir) = server_with_skill();
    let req = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize"});
    let resp = server.handle(&req).unwrap();
    assert_eq!(resp["result"]["serverInfo"]["name"], "keel");
    assert!(resp["result"]["capabilities"]["tools"].is_object());
}

#[test]
fn tools_list_exposes_the_convergence_tools() {
    let (mut server, _dir) = server_with_skill();
    let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let resp = server.handle(&req).unwrap();
    let names: Vec<&str> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"keel.skills.list"));
    assert!(names.contains(&"keel.skills.load"));
    assert!(names.contains(&"keel.rules.query"));
}

#[test]
fn list_then_load_delivers_content_and_records_a_receipt() {
    let (mut server, _dir) = server_with_skill();

    let listing = call(&mut server, "keel.skills.list", json!({}));
    assert!(listing.contains("access-patterns"));
    assert!(listing.contains("not-loaded"));

    let loaded = call(
        &mut server,
        "keel.skills.load",
        json!({ "id": "access-patterns" }),
    );
    assert!(
        loaded.contains("USE the query builder"),
        "the skill content is delivered: {loaded}"
    );

    // Keel — not the model's word — recorded the delivery.
    let consumed = server.store.consumed_skill_ids("session-test").unwrap();
    assert!(consumed.contains(&"access-patterns".to_string()));

    // Second list reflects the loaded state.
    let relisting = call(&mut server, "keel.skills.list", json!({}));
    assert!(relisting.contains("loaded:Compact"), "state: {relisting}");
}

/// With no agent declared, invoke reports it honestly rather than pretending.
/// The full local-CLI routing + outputSchema validation is exercised
/// end-to-end over stdio in `test/tests/mcp_stdio.rs`.
#[test]
fn agent_invoke_reports_an_undeclared_agent() {
    let (mut server, _dir) = server_with_skill();
    let out = call(
        &mut server,
        "keel.agent.invoke",
        json!({ "agent": "auditor" }),
    );
    assert!(
        out.contains("not declared"),
        "an undeclared agent is reported, not faked: {out}"
    );
}

#[test]
fn unknown_method_is_a_jsonrpc_error_not_a_crash() {
    let (mut server, _dir) = server_with_skill();
    let req = json!({"jsonrpc": "2.0", "id": 9, "method": "nonsense"});
    let resp = server.handle(&req).unwrap();
    assert_eq!(resp["error"]["code"], -32601);
}

/// A validated verdict becomes the `task.completed` text the audit gate keys on:
/// record-audit matches the literal `VERDICT: GO`, so `{"verdict":"GO"}` must
/// render as `VERDICT: GO`. Missing verdict → nothing to record.
#[test]
fn verdict_event_content_derives_gate_text_from_validated_output() {
    let go = json!({ "verdict": "GO", "note": "ship it" });
    assert_eq!(
        verdict_event_content(&go).as_deref(),
        Some("VERDICT: GO\nship it")
    );

    let bare = json!({ "verdict": "NO-GO" });
    assert_eq!(verdict_event_content(&bare).as_deref(), Some("VERDICT: NO-GO"));

    let missing = json!({ "note": "no verdict field" });
    assert!(verdict_event_content(&missing).is_none());
}
