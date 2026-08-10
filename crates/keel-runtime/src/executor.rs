// SPDX-License-Identifier: Apache-2.0
//! The provider-facing boundary of the governed runtime.
//!
//! Keel does NOT call model provider APIs (D-012): the runtime governs the
//! model's local execution environment, it does not become the model's API
//! client. The only concrete executor is a LOCAL CLI (`CliModelExecutor`,
//! F4b) — keel runs a governed command and treats its stdout as the response.
//! `MockModelExecutor` is the deterministic test double.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelMessage {
    pub role: String,
    pub content: String,
}

impl ModelMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRequest {
    pub session_id: String,
    pub messages: Vec<ModelMessage>,
    #[serde(default)]
    pub context: Vec<String>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
}

impl ModelRequest {
    pub fn new(session_id: impl Into<String>, messages: Vec<ModelMessage>) -> Self {
        Self {
            session_id: session_id.into(),
            messages,
            context: Vec::new(),
            tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelResponse {
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    pub provider_id: String,
    pub model_id: String,
}

impl ModelResponse {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            tool_calls: Vec::new(),
            provider_id: "mock".to_string(),
            model_id: "mock".to_string(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ExecutorError {
    #[error("executor is closed")]
    Closed,
    #[error("executor error: {0}")]
    Failed(String),
}

/// The only provider-facing contract in the governed runtime. Implemented by
/// a local CLI (`CliModelExecutor`) and the test double below — never by an
/// HTTP client to a model provider (D-012).
pub trait ModelExecutor {
    fn provider_id(&self) -> &str;
    fn model_id(&self) -> &str;
    fn complete(&mut self, request: ModelRequest) -> Result<ModelResponse, ExecutorError>;
    fn cancel(&mut self, session_id: &str) -> Result<(), ExecutorError>;
}

#[derive(Debug, Default)]
pub struct MockModelExecutor {
    responses: VecDeque<ModelResponse>,
    requests: Vec<ModelRequest>,
    closed: bool,
}

impl MockModelExecutor {
    pub fn with_response(response: ModelResponse) -> Self {
        Self {
            responses: VecDeque::from([response]),
            ..Self::default()
        }
    }

    pub fn requests(&self) -> &[ModelRequest] {
        &self.requests
    }

    pub fn from_responses(responses: impl IntoIterator<Item = ModelResponse>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            ..Self::default()
        }
    }
}

impl ModelExecutor for MockModelExecutor {
    fn provider_id(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        "mock"
    }

    fn complete(&mut self, request: ModelRequest) -> Result<ModelResponse, ExecutorError> {
        if self.closed {
            return Err(ExecutorError::Closed);
        }
        self.requests.push(request);
        self.responses
            .pop_front()
            .ok_or_else(|| ExecutorError::Failed("mock response queue is empty".to_string()))
    }

    fn cancel(&mut self, _session_id: &str) -> Result<(), ExecutorError> {
        self.closed = true;
        Ok(())
    }
}

/// A local CLI as a model executor (D-012): keel runs a governed command and
/// treats its stdout as the model's response. The prompt (context + messages)
/// is written to the command's stdin; the command runs confined to the
/// workspace (`cwd = root`, `env_clear` + `PATH` only) so an agent inherits no
/// ambient secrets. This is the ONLY non-mock executor — keel never speaks a
/// provider API.
pub struct CliModelExecutor {
    command: Vec<String>,
    root: PathBuf,
    provider_id: String,
    model_id: String,
    /// Extra env vars set on the child AFTER `env_clear` (which still strips the
    /// ambient environment). Declared per-executor in `config.env`; the only way
    /// to hand a governed CLI what it needs to run (e.g. `HOME` for its auth
    /// config) without inheriting ambient secrets wholesale.
    env: Vec<(String, String)>,
}

impl CliModelExecutor {
    /// `command` is the argv of the CLI (e.g. `["codex", "exec", "--json"]`);
    /// `root` is the workspace it runs in; `id` labels the executor in
    /// evidence.
    pub fn new(command: Vec<String>, root: impl Into<PathBuf>, id: impl Into<String>) -> Self {
        let model_id = id.into();
        Self {
            command,
            root: root.into(),
            provider_id: "cli".to_string(),
            model_id,
            env: Vec::new(),
        }
    }

    /// Declares extra env vars for the child (from `config.env`). `env_clear`
    /// still runs first, so only `PATH` plus these reach the CLI.
    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.env = env;
        self
    }

    fn prompt(request: &ModelRequest) -> String {
        let mut sections: Vec<String> = request.context.clone();
        for message in &request.messages {
            sections.push(message.content.clone());
        }
        sections.join("\n\n")
    }
}

impl ModelExecutor for CliModelExecutor {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn complete(&mut self, request: ModelRequest) -> Result<ModelResponse, ExecutorError> {
        let (program, args) = self
            .command
            .split_first()
            .ok_or_else(|| ExecutorError::Failed("executor command is empty".to_string()))?;

        let mut process = Command::new(program);
        process
            .args(args)
            .current_dir(&self.root)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = std::env::var_os("PATH") {
            process.env("PATH", path);
        }
        for (name, value) in &self.env {
            process.env(name, value);
        }

        let mut child = process
            .spawn()
            .map_err(|error| ExecutorError::Failed(format!("spawn `{program}`: {error}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(Self::prompt(&request).as_bytes());
            // drop closes stdin → the CLI sees EOF.
        }
        let output = child
            .wait_with_output()
            .map_err(|error| ExecutorError::Failed(error.to_string()))?;
        if !output.status.success() {
            return Err(ExecutorError::Failed(format!(
                "executor `{}` exited with {}: {}",
                self.model_id,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(ModelResponse {
            content: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            tool_calls: Vec::new(),
            provider_id: self.provider_id.clone(),
            model_id: self.model_id.clone(),
        })
    }

    fn cancel(&mut self, _session_id: &str) -> Result<(), ExecutorError> {
        // Each `complete` is a fresh short-lived process; there is nothing to
        // cancel between turns.
        Ok(())
    }
}

/// Resolves the argv of a `model-executor` component's `config.command`. The
/// snapshot stores executors under the key `model-executor:<id>`.
pub fn executor_command(
    components: &std::collections::BTreeMap<String, keel_engine::snapshot::CompiledComponent>,
    executor_id: &str,
) -> Result<Vec<String>, ExecutorError> {
    let key = format!("model-executor:{executor_id}");
    let component = components.get(&key).ok_or_else(|| {
        ExecutorError::Failed(format!("executor `{executor_id}` is not in the snapshot"))
    })?;
    let command = component
        .config
        .as_ref()
        .and_then(|c| c.get("command"))
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            ExecutorError::Failed(format!(
                "executor `{executor_id}` has no `command` — a governed executor is a local CLI (D-012)"
            ))
        })?;
    let argv = command
        .iter()
        .map(|v| {
            v.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                ExecutorError::Failed("executor command entries must be strings".to_string())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if argv.is_empty() {
        return Err(ExecutorError::Failed(format!(
            "executor `{executor_id}` has an empty command"
        )));
    }
    Ok(argv)
}

/// Resolves a `config.env` map into concrete name/value pairs for a child
/// process. A value of the exact form `${NAME}` inherits `NAME` from keel's
/// own environment — so `env: { HOME: "${HOME}" }` passes the operator's HOME
/// through without inheriting the whole environment. Any other value is
/// literal. A missing `${NAME}` resolves to empty (declared-but-unset is not
/// an error). Shared by `executor_env` (`kind: ModelExecutor`) and
/// `compiled_mcp_providers` (`kind: MCPProvider`, H-011) — the same `${VAR}`
/// convention, one resolver.
pub(crate) fn resolve_env_map(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(name, value)| {
            let raw = value.as_str()?;
            let resolved = match raw.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
                Some(var) => std::env::var(var).unwrap_or_default(),
                None => raw.to_string(),
            };
            Some((name.clone(), resolved))
        })
        .collect()
}

/// Resolves a `model-executor` component's `config.env` map into concrete
/// name/value pairs for the child (see `resolve_env_map`). Absent
/// `config.env` yields an empty vec (the pre-existing PATH-only behavior).
pub fn executor_env(
    components: &std::collections::BTreeMap<String, keel_engine::snapshot::CompiledComponent>,
    executor_id: &str,
) -> Vec<(String, String)> {
    let key = format!("model-executor:{executor_id}");
    let Some(map) = components
        .get(&key)
        .and_then(|c| c.config.as_ref())
        .and_then(|c| c.get("env"))
        .and_then(|e| e.as_object())
    else {
        return Vec::new();
    };
    resolve_env_map(map)
}

#[cfg(test)]
mod executor_env_tests {
    use super::executor_env;
    use keel_engine::snapshot::CompiledComponent;
    use std::collections::BTreeMap;

    fn executor_with_config(config: serde_json::Value) -> BTreeMap<String, CompiledComponent> {
        let mut m = BTreeMap::new();
        m.insert(
            "model-executor:x".to_string(),
            CompiledComponent {
                kind: "ModelExecutor".into(),
                id: "x".into(),
                version: "0".into(),
                description: None,
                match_: Default::default(),
                content: None,
                inline: None,
                requirements: vec![],
                capabilities: vec![],
                config: Some(config),
            },
        );
        m
    }

    #[test]
    fn resolves_literal_and_dollar_var_and_ignores_missing_env() {
        // `${VAR}` inherits from keel's own environment; a plain value is
        // literal; an absent config.env yields nothing.
        // SAFETY: single-threaded unit test setting a private, test-only var.
        unsafe {
            std::env::set_var("KEEL_TEST_HOME", "/home/keeltest");
        }
        let components = executor_with_config(serde_json::json!({
            "command": ["claude", "-p"],
            "env": { "HOME": "${KEEL_TEST_HOME}", "MODE": "batch", "GONE": "${KEEL_TEST_UNSET_XYZ}" }
        }));
        let mut env = executor_env(&components, "x");
        env.sort();
        assert_eq!(
            env,
            vec![
                ("GONE".to_string(), String::new()), // unset ${VAR} → empty, not an error
                ("HOME".to_string(), "/home/keeltest".to_string()),
                ("MODE".to_string(), "batch".to_string()),
            ]
        );

        // No config.env → empty (preserves the PATH-only behavior).
        let none = executor_with_config(serde_json::json!({ "command": ["claude"] }));
        assert!(executor_env(&none, "x").is_empty());
    }
}
