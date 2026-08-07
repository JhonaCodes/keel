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
