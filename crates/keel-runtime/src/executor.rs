use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::Duration;
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

/// The only provider-facing contract in the governed runtime.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpProvider {
    Anthropic,
    OpenAi,
}

pub struct HttpModelExecutor {
    provider: HttpProvider,
    model: String,
    endpoint: String,
    api_key: String,
    client: reqwest::blocking::Client,
    closed: bool,
}

impl HttpModelExecutor {
    pub fn new(
        provider: HttpProvider,
        model: impl Into<String>,
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Result<Self, ExecutorError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| ExecutorError::Failed(error.to_string()))?;
        Ok(Self {
            provider,
            model: model.into(),
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            client,
            closed: false,
        })
    }

    pub fn anthropic(
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Result<Self, ExecutorError> {
        Self::new(
            HttpProvider::Anthropic,
            model,
            "https://api.anthropic.com/v1/messages",
            api_key,
        )
    }

    pub fn openai(
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Result<Self, ExecutorError> {
        Self::new(
            HttpProvider::OpenAi,
            model,
            "https://api.openai.com/v1/responses",
            api_key,
        )
    }

    fn complete_anthropic(&self, request: ModelRequest) -> Result<ModelResponse, ExecutorError> {
        let messages = request
            .messages
            .iter()
            .filter(|message| message.role != "system")
            .map(|message| serde_json::json!({"role": message.role, "content": message.content}))
            .collect::<Vec<_>>();
        let system = request
            .messages
            .iter()
            .filter(|message| message.role == "system")
            .map(|message| message.content.as_str())
            .chain(request.context.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": messages,
        });
        if !system.is_empty() {
            body["system"] = serde_json::Value::String(system);
        }
        if !request.tools.is_empty() {
            body["tools"] = serde_json::Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        serde_json::json!({
                            "name": tool.name,
                            "description": tool.description,
                            "input_schema": tool.input_schema,
                        })
                    })
                    .collect(),
            );
        }
        let response = self
            .client
            .post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .map_err(|error| ExecutorError::Failed(error.to_string()))?;
        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .map_err(|error| ExecutorError::Failed(error.to_string()))?;
        if !status.is_success() {
            return Err(ExecutorError::Failed(format!(
                "Anthropic HTTP {status}: {value}"
            )));
        }
        Ok(parse_anthropic_response(&value, &self.model))
    }

    fn complete_openai(&self, request: ModelRequest) -> Result<ModelResponse, ExecutorError> {
        let mut input = request
            .messages
            .iter()
            .map(|message| serde_json::json!({"role": message.role, "content": message.content}))
            .collect::<Vec<_>>();
        if !request.context.is_empty() {
            input.insert(
                0,
                serde_json::json!({"role": "system", "content": request.context.join("\n\n")}),
            );
        }
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                })
            })
            .collect::<Vec<_>>();
        let mut body = serde_json::json!({"model": self.model, "input": input});
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools);
        }
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(|error| ExecutorError::Failed(error.to_string()))?;
        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .map_err(|error| ExecutorError::Failed(error.to_string()))?;
        if !status.is_success() {
            return Err(ExecutorError::Failed(format!(
                "OpenAI HTTP {status}: {value}"
            )));
        }
        Ok(parse_openai_response(&value, &self.model))
    }
}

impl ModelExecutor for HttpModelExecutor {
    fn provider_id(&self) -> &str {
        match self.provider {
            HttpProvider::Anthropic => "anthropic",
            HttpProvider::OpenAi => "openai",
        }
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn complete(&mut self, request: ModelRequest) -> Result<ModelResponse, ExecutorError> {
        if self.closed {
            return Err(ExecutorError::Closed);
        }
        match self.provider {
            HttpProvider::Anthropic => self.complete_anthropic(request),
            HttpProvider::OpenAi => self.complete_openai(request),
        }
    }

    fn cancel(&mut self, _session_id: &str) -> Result<(), ExecutorError> {
        self.closed = true;
        Ok(())
    }
}

fn parse_anthropic_response(value: &serde_json::Value, model: &str) -> ModelResponse {
    let content = value["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["type"] == "text")
        .filter_map(|item| item["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let tool_calls = value["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["type"] == "tool_use")
        .filter_map(|item| {
            Some(ToolCall {
                name: item["name"].as_str()?.to_string(),
                arguments: item["input"].clone(),
            })
        })
        .collect();
    ModelResponse {
        content,
        tool_calls,
        provider_id: "anthropic".to_string(),
        model_id: value["model"].as_str().unwrap_or(model).to_string(),
    }
}

fn parse_openai_response(value: &serde_json::Value, model: &str) -> ModelResponse {
    let output = value["output"].as_array().cloned().unwrap_or_default();
    let content = output
        .iter()
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter(|item| item["type"] == "output_text")
        .filter_map(|item| item["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let tool_calls = output
        .iter()
        .filter(|item| item["type"] == "function_call")
        .filter_map(|item| {
            let arguments = item["arguments"]
                .as_str()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_else(|| item["arguments"].clone());
            Some(ToolCall {
                name: item["name"].as_str()?.to_string(),
                arguments,
            })
        })
        .collect();
    ModelResponse {
        content,
        tool_calls,
        provider_id: "openai".to_string(),
        model_id: value["model"].as_str().unwrap_or(model).to_string(),
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

#[cfg(test)]
mod provider_tests {
    use super::{parse_anthropic_response, parse_openai_response};

    #[test]
    fn parses_anthropic_text_and_tool_use() {
        let response = parse_anthropic_response(
            &serde_json::json!({
                "model": "claude-test",
                "content": [
                    {"type": "text", "text": "ready"},
                    {"type": "tool_use", "name": "skill.read", "input": {"skill_id": "rust"}}
                ]
            }),
            "fallback",
        );
        assert_eq!(response.content, "ready");
        assert_eq!(response.tool_calls[0].name, "skill.read");
    }

    #[test]
    fn parses_openai_text_and_function_call() {
        let response = parse_openai_response(
            &serde_json::json!({
                "model": "gpt-test",
                "output": [
                    {"type": "message", "content": [{"type": "output_text", "text": "ready"}]},
                    {"type": "function_call", "name": "skill.read", "arguments": "{\"skill_id\":\"rust\"}"}
                ]
            }),
            "fallback",
        );
        assert_eq!(response.content, "ready");
        assert_eq!(response.tool_calls[0].arguments["skill_id"], "rust");
    }
}
