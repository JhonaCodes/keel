use crate::{AgentScheduler, ModelExecutor, ModelMessage, ModelRequest, RuntimeError};
use keel_engine::snapshot::Snapshot;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone)]
struct AgentRoute {
    executor_id: String,
    objective: Option<String>,
    output_schema: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentResult {
    pub agent_id: String,
    pub executor_id: String,
    pub content: String,
    pub output: Option<serde_json::Value>,
}

#[derive(Debug, Error)]
pub enum AgentBrokerError {
    #[error("agent `{0}` is not declared in the snapshot")]
    AgentNotFound(String),
    #[error("scheduler returned no claim for the submitted agent")]
    ClaimLost,
    #[error("agent result is not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("agent result violates its contract")]
    InvalidContract,
    #[error("could not read agent contract: {0}")]
    ContractIo(#[from] std::io::Error),
    #[error("agent contract is invalid: {0}")]
    ContractSchema(String),
    #[error(transparent)]
    Scheduler(#[from] crate::SchedulerError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
}

/// Returns the last top-level `{...}` object in `content`, if any. Brace
/// matching is string-aware, so braces inside JSON strings do not break the
/// span — this lets a governed agent print a prose report and END with the
/// JSON object its `outputSchema` requires.
fn extract_last_json_object(content: &str) -> Option<&str> {
    let bytes = content.as_bytes();
    let mut depth: i32 = 0;
    let mut start: Option<usize> = None;
    let mut best: Option<(usize, usize)> = None;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    best = start.take().map(|s| (s, i + 1));
                }
            }
            _ => {}
        }
    }
    best.map(|(s, e)| &content[s..e])
}

pub struct AgentBroker {
    routes: BTreeMap<String, AgentRoute>,
    workspace_root: PathBuf,
}

impl AgentBroker {
    pub fn from_snapshot(snapshot: &Snapshot, workspace_root: &Path) -> Self {
        let routes = snapshot
            .agents
            .values()
            .map(|agent| {
                (
                    agent.id.clone(),
                    AgentRoute {
                        executor_id: agent.executor.clone(),
                        objective: agent.objective.clone(),
                        output_schema: agent.output_schema.as_ref().map(PathBuf::from),
                    },
                )
            })
            .collect();
        Self {
            routes,
            workspace_root: workspace_root.to_path_buf(),
        }
    }

    pub fn executor_for(&self, agent_id: &str) -> Option<&str> {
        self.routes
            .get(agent_id)
            .map(|route| route.executor_id.as_str())
    }

    pub fn has_agents(&self) -> bool {
        !self.routes.is_empty()
    }

    pub fn invoke<E: ModelExecutor>(
        &self,
        session_id: &str,
        agent_id: &str,
        input: &str,
        scheduler: &mut AgentScheduler,
        executor: &mut E,
    ) -> Result<AgentResult, AgentBrokerError> {
        let route = self
            .routes
            .get(agent_id)
            .ok_or_else(|| AgentBrokerError::AgentNotFound(agent_id.to_string()))?;
        let submitted = scheduler.submit(session_id, agent_id, &route.executor_id)?;
        let claimed = scheduler
            .claim_task(&submitted.id)?
            .ok_or(AgentBrokerError::ClaimLost)?;
        scheduler.start(&claimed.id)?;
        let execution = (|| {
            // Read the output contract once: it is used BOTH to steer the model
            // (inject the schema into the prompt) and to validate the reply. keel
            // itself never calls a provider API (D-012) — the schema travels in
            // the prompt so the LOCAL CLI knows to emit JSON on its own.
            let schema = match &route.output_schema {
                Some(schema_path) => {
                    let raw = std::fs::read_to_string(self.workspace_root.join(schema_path))?;
                    Some(serde_json::from_str::<serde_json::Value>(&raw)?)
                }
                None => None,
            };
            let base_prompt = match &route.objective {
                Some(objective) => format!("Objective: {objective}\n\nInput:\n{input}"),
                None => input.to_string(),
            };
            let prompt = match &schema {
                Some(schema) => format!(
                    "{base_prompt}\n\n---\nWhen you are done, end your reply with a single line \
                     containing ONLY a JSON object that matches this JSON Schema — no prose, \
                     no code fences, and nothing after it:\n{}",
                    serde_json::to_string(schema).unwrap_or_default()
                ),
                None => base_prompt,
            };
            let response = executor
                .complete(ModelRequest::new(
                    session_id,
                    vec![ModelMessage::user(prompt)],
                ))
                .map_err(|error| RuntimeError::Executor {
                    message: error.to_string(),
                })?;
            let output = if let Some(schema) = &schema {
                let validator = jsonschema::validator_for(schema)
                    .map_err(|error| AgentBrokerError::ContractSchema(error.to_string()))?;
                // The reply may be a prose report ending with the JSON verdict;
                // take the trailing JSON object. Falling back to the whole reply
                // preserves the "empty stdout" error (serde: line 1 column 1).
                let json_str = extract_last_json_object(&response.content)
                    .unwrap_or(response.content.as_str());
                let value: serde_json::Value = serde_json::from_str(json_str)?;
                if !validator.is_valid(&value) {
                    return Err(AgentBrokerError::InvalidContract);
                }
                Some(value)
            } else {
                None
            };
            Ok(AgentResult {
                agent_id: agent_id.to_string(),
                executor_id: route.executor_id.clone(),
                content: response.content,
                output,
            })
        })();
        match execution {
            Ok(result) => {
                scheduler.complete(&claimed.id)?;
                Ok(result)
            }
            Err(error) => {
                scheduler.fail(&claimed.id)?;
                Err(error)
            }
        }
    }
}
