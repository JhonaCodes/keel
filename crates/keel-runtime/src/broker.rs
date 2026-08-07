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
            let prompt = match &route.objective {
                Some(objective) => format!("Objective: {objective}\n\nInput:\n{input}"),
                None => input.to_string(),
            };
            let response = executor
                .complete(ModelRequest::new(
                    session_id,
                    vec![ModelMessage::user(prompt)],
                ))
                .map_err(|error| RuntimeError::Executor {
                    message: error.to_string(),
                })?;
            let output = if let Some(schema_path) = &route.output_schema {
                let raw_schema = std::fs::read_to_string(self.workspace_root.join(schema_path))?;
                let schema: serde_json::Value = serde_json::from_str(&raw_schema)?;
                let validator = jsonschema::validator_for(&schema)
                    .map_err(|error| AgentBrokerError::ContractSchema(error.to_string()))?;
                let value: serde_json::Value = serde_json::from_str(&response.content)?;
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
