// SPDX-License-Identifier: Apache-2.0
//! `keel mcp` — the convergence plane (spec section 12, section 9).
//!
//! Keel is the single point every model converges on: instead of each CLI
//! reading skills and agents from its OWN configuration, the child asks KEEL.
//! keel launches this server as an MCP endpoint wired into the child at spawn
//! time, so the model KNOWS it has skills and pulls their content into its
//! context in the moment it needs them — through keel, which records what was
//! delivered.
//!
//! Transport: MCP stdio — newline-delimited JSON-RPC 2.0 (one message per
//! line). The protocol subset keel needs (`initialize`, `tools/list`,
//! `tools/call`) is small enough to serve by hand with serde_json — no SDK,
//! no async, no new dependency.
//!
//! This plane is CONVERGENCE, not enforcement: if the child never calls it,
//! nothing breaks — the hard rings (shims, sandbox) are independent and
//! always active. That separation is deliberate (D-012): P1 never depends on
//! P2's cooperation.

use anyhow::{Context, Result};
use keel_engine::session::{SessionStore, deliver_skills};
use keel_engine::snapshot::Snapshot;
use keel_engine::workspace::WorkspaceFiles;
use keel_runtime::{
    AgentBroker, AgentScheduler, CliModelExecutor, RuntimeStore, SkillReadReceipt,
    executor_command, executor_env,
};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL_VERSION: &str = "2024-11-05";
const RUNTIME_DB: &str = "runtime.sqlite";
const SCHEDULER_DB: &str = "scheduler.sqlite";
const MAX_CONCURRENT_AGENTS: u32 = 4;

/// Runs the stdio MCP server for `session_id` against the workspace at `root`
/// until stdin closes. Reads line-delimited JSON-RPC from stdin, writes
/// responses to stdout; diagnostics go to stderr (stdout is the protocol
/// channel and must stay clean).
pub fn serve(root: &Path, session_id: &str) -> Result<()> {
    let files = WorkspaceFiles::empty(root.to_path_buf());
    let snapshot = Snapshot::load(&files.snapshot_path())
        .context("no published snapshot — run `keel compile` first")?;
    let store = RuntimeStore::open(&files.state_dir().join(RUNTIME_DB))?;
    // A governed session is registered before any receipt (the receipt has a
    // foreign key to the session). Idempotent against the same snapshot hash.
    store.ensure_session(session_id, &snapshot.hash.to_string())?;
    let mut server = Server {
        root: root.to_path_buf(),
        snapshot,
        sessions: SessionStore::new(&files.state_dir()),
        store,
        session_id: session_id.to_string(),
    };

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line.context("mcp: reading stdin")?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                // A malformed line is a protocol error for that message, never
                // a crash: report and keep serving.
                eprintln!("[keel mcp] invalid JSON-RPC line: {e}");
                continue;
            }
        };
        // Notifications (no `id`) get no response by JSON-RPC rule.
        let Some(response) = server.handle(&request) else {
            continue;
        };
        let mut out = stdout.lock();
        serde_json::to_writer(&mut out, &response)?;
        out.write_all(b"\n")?;
        out.flush()?;
    }
    Ok(())
}

struct Server {
    root: PathBuf,
    snapshot: Snapshot,
    sessions: SessionStore,
    store: RuntimeStore,
    session_id: String,
}

impl Server {
    /// Dispatches one JSON-RPC request, returning the response (or `None` for
    /// a notification).
    fn handle(&mut self, request: &Value) -> Option<Value> {
        // Notifications carry no id and expect no reply (JSON-RPC): `?` drops
        // them here.
        let id = request.get("id").cloned()?;
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        let result = match method {
            "initialize" => Ok(self.initialize()),
            "tools/list" => Ok(self.tools_list()),
            "tools/call" => self.tools_call(&params),
            other => Err(rpc_error(-32601, &format!("method not found: {other}"))),
        };

        Some(match result {
            Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
            Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
        })
    }

    fn initialize(&self) -> Value {
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "keel", "version": env!("CARGO_PKG_VERSION") },
        })
    }

    fn tools_list(&self) -> Value {
        json!({
            "tools": [
                {
                    "name": "keel.skills.list",
                    "description": "List the skills Keel governs for this session, with their loaded state. Call this to discover what you can pull into context.",
                    "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
                },
                {
                    "name": "keel.skills.load",
                    "description": "Load a governed skill's content into your context by id. Keel records the delivery. Use the id from keel.skills.list.",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "id": { "type": "string" } },
                        "required": ["id"],
                        "additionalProperties": false
                    }
                },
                {
                    "name": "keel.rules.query",
                    "description": "Ask which governed rules apply to a command or file path, so you can act within them BEFORE trying. Advisory — it never blocks.",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "command": { "type": "string" }, "path": { "type": "string" } },
                        "additionalProperties": false
                    }
                },
                {
                    "name": "keel.agent.invoke",
                    "description": "Ask Keel to run a governed agent (possibly on another model) and return its feedback. Cross-model, deterministic.",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "agent": { "type": "string" }, "input": { "type": "string" } },
                        "required": ["agent"],
                        "additionalProperties": false
                    }
                }
            ]
        })
    }

    fn tools_call(&mut self, params: &Value) -> Result<Value, Value> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_error(-32602, "missing tool name"))?;
        let args = params.get("arguments").cloned().unwrap_or(Value::Null);

        match name {
            "keel.skills.list" => Ok(text_result(&self.skills_list())),
            "keel.skills.load" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| rpc_error(-32602, "missing skill id"))?;
                Ok(text_result(&self.skills_load(id)))
            }
            "keel.rules.query" => {
                let command = args.get("command").and_then(Value::as_str);
                let path = args.get("path").and_then(Value::as_str);
                Ok(text_result(&self.rules_query(command, path)))
            }
            "keel.agent.invoke" => {
                let agent = args
                    .get("agent")
                    .and_then(Value::as_str)
                    .ok_or_else(|| rpc_error(-32602, "missing agent id"))?;
                let input = args.get("input").and_then(Value::as_str).unwrap_or("");
                Ok(text_result(&self.agent_invoke(agent, input)))
            }
            other => Err(rpc_error(-32602, &format!("unknown tool: {other}"))),
        }
    }

    fn skills_list(&self) -> String {
        let loaded = self.sessions.load(&self.session_id).loaded_skills;
        if self.snapshot.skills.is_empty() {
            return "No skills are governed in this workspace.".to_string();
        }
        let mut out = String::from("Governed skills (call keel.skills.load with an id):\n");
        for (id, skill) in &self.snapshot.skills {
            let state = match loaded.get(id) {
                Some(level) => format!("loaded:{level:?}"),
                None => "not-loaded".to_string(),
            };
            out.push_str(&format!("- {id} (v{}) [{state}]\n", skill.version));
        }
        out
    }

    fn skills_load(&mut self, id: &str) -> String {
        let mut state = self.sessions.load(&self.session_id);
        let refs = vec![format!("skill:{id}")];
        let payload = deliver_skills(&self.snapshot, &self.root, &mut state, &refs, false, false);
        // Record the delivery: Keel, not the model's word, is the evidence
        // that a skill entered context (D-003, spec section 6.4). Best-effort;
        // a store hiccup must not deny the model its content.
        if let Some(skill) = self.snapshot.skills.get(id) {
            let receipt = SkillReadReceipt {
                skill_id: id.to_string(),
                version: skill.version.clone(),
                content_hash: String::new(),
                content: String::new(),
                receipt_id: format!("rcpt_{}", ulid::Ulid::new().to_string().to_lowercase()),
                required: false,
                session_id: self.session_id.clone(),
                phase: "convergence".to_string(),
                reason: Some("mcp keel.skills.load".to_string()),
                read_at: keel_engine::ledger::now_ts(),
            };
            if let Err(e) = self.store.append_skill_receipt(&receipt) {
                eprintln!("[keel mcp] receipt not recorded for {id}: {e}");
            }
        }
        let _ = self
            .sessions
            .save(&self.session_id, &state, &keel_engine::ledger::now_ts());
        payload.join("\n\n")
    }

    fn rules_query(&self, command: Option<&str>, path: Option<&str>) -> String {
        // Advisory surface: name the rules whose scope could touch this action,
        // so the model can steer before acting. It NEVER evaluates or blocks
        // here (that is the broker's job at the moment of the action).
        let mut hits = Vec::new();
        for rule in &self.snapshot.rules {
            let touches_path = path
                .and_then(|p| rule.scope.as_ref().map(|s| s.matches(Some(p), None)))
                .unwrap_or(false);
            let mentions_command = command.is_some()
                && rule
                    .on
                    .iter()
                    .any(|k| matches!(k, keel_core::event::EventKind::CommandRequested));
            if touches_path || mentions_command {
                hits.push(format!("- {} (on: {:?})", rule.id, rule.on));
            }
        }
        if hits.is_empty() {
            "No governed rule matches that command/path (still subject to the OS containment)."
                .to_string()
        } else {
            format!("Governed rules that may apply:\n{}", hits.join("\n"))
        }
    }

    /// Runs a governed agent and returns its feedback — cross-model by design:
    /// the agent's executor is a LOCAL CLI (e.g. `codex exec`), so a session on
    /// one model can get an audit from another, deterministically and without
    /// any provider API (D-012). The scheduler leases the task; the output is
    /// validated against the agent's `outputSchema` (invariant 12) before it is
    /// trusted; evidence is keel's.
    fn agent_invoke(&self, agent_id: &str, input: &str) -> String {
        let broker = AgentBroker::from_snapshot(&self.snapshot, &self.root);
        let Some(executor_id) = broker.executor_for(agent_id).map(ToOwned::to_owned) else {
            return format!("agent `{agent_id}` is not declared in this workspace.");
        };
        let command = match executor_command(&self.snapshot.components, &executor_id) {
            Ok(c) => c,
            Err(e) => return format!("cannot run agent `{agent_id}`: {e}"),
        };
        let env = executor_env(&self.snapshot.components, &executor_id);
        let mut scheduler = match AgentScheduler::open(
            &WorkspaceFiles::empty(self.root.clone())
                .state_dir()
                .join(SCHEDULER_DB),
            MAX_CONCURRENT_AGENTS,
        ) {
            Ok(s) => s,
            Err(e) => return format!("scheduler unavailable: {e}"),
        };
        let mut executor =
            CliModelExecutor::new(command, self.root.clone(), executor_id).with_env(env);
        match broker.invoke(
            &self.session_id,
            agent_id,
            input,
            &mut scheduler,
            &mut executor,
        ) {
            Ok(result) => match result.output {
                Some(value) => format!(
                    "agent `{agent_id}` (executor `{}`) — validated output:\n{}",
                    result.executor_id,
                    serde_json::to_string_pretty(&value).unwrap_or(result.content)
                ),
                None => format!(
                    "agent `{agent_id}` (executor `{}`):\n{}",
                    result.executor_id, result.content
                ),
            },
            Err(e) => format!("agent `{agent_id}` failed: {e}"),
        }
    }
}

/// Wraps text as an MCP `tools/call` result.
fn text_result(text: &str) -> Value {
    json!({ "content": [ { "type": "text", "text": text } ] })
}

fn rpc_error(code: i64, message: &str) -> Value {
    json!({ "code": code, "message": message })
}

#[cfg(test)]
#[path = "../tests-unit/mcp.rs"]
mod tests;
