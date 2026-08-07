// SPDX-License-Identifier: Apache-2.0
//! CLI orchestration for Keel-owned governed sessions.

use crate::commands;
use anyhow::{Context, Result, bail};
use keel_engine::lock::Lock;
use keel_engine::snapshot::Snapshot;
use keel_engine::workspace::WorkspaceFiles;
use keel_runtime::{
    AgentBroker, AgentScheduler, ArtifactKind, CapabilityManager, CapabilityRequest,
    ComponentReadRequest, HttpModelExecutor, HttpProvider, MockModelExecutor, ModelExecutor,
    ModelMessage, ModelRequest, ModelResponse, Operation, Phase, RuntimeHost, RuntimeStore,
    SkillReadRequest, ToolCall, ToolDefinition,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

const CONFIG_FILE: &str = "runtime-config.json";
const RUNTIME_DB: &str = "runtime.sqlite";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExecutorConfig {
    provider: String,
    model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential: Option<CredentialRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "kebab-case")]
enum CredentialRef {
    Environment { variable: String },
    OsKeychain { account: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeConfig {
    default_executor: String,
    executors: BTreeMap<String, ExecutorConfig>,
}

struct SessionServices<'a> {
    capabilities: &'a mut CapabilityManager,
    broker: &'a AgentBroker,
    scheduler: &'a mut AgentScheduler,
    config: &'a RuntimeConfig,
}

impl RuntimeConfig {
    fn mock() -> Self {
        Self {
            default_executor: "mock".to_string(),
            executors: BTreeMap::from([(
                "mock".to_string(),
                ExecutorConfig {
                    provider: "mock".to_string(),
                    model: "mock".to_string(),
                    endpoint: None,
                    credential: None,
                },
            )]),
        }
    }

    fn load(root: &Path) -> Result<Self> {
        let path = state_dir(root).join(CONFIG_FILE);
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("missing runtime configuration at {}", path.display()))?;
        serde_json::from_str(&raw).context("invalid runtime configuration")
    }

    fn save(&self, root: &Path) -> Result<()> {
        let path = state_dir(root).join(CONFIG_FILE);
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

pub struct ExecutorConfiguration {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub endpoint: Option<String>,
    pub credential_env: Option<String>,
    pub api_key_stdin: bool,
    pub json_output: bool,
}

pub fn configure_executor_add(root: &Path, request: ExecutorConfiguration) -> Result<ExitCode> {
    let ExecutorConfiguration {
        id,
        provider,
        model,
        endpoint,
        credential_env,
        api_key_stdin,
        json_output,
    } = request;
    if !matches!(provider.as_str(), "mock" | "anthropic" | "openai") {
        bail!("unsupported provider `{provider}`; expected mock, anthropic or openai");
    }
    if credential_env.is_some() && api_key_stdin {
        bail!("choose either --credential-env or --api-key-stdin");
    }
    if provider != "mock" && credential_env.is_none() && !api_key_stdin {
        bail!("real providers require --credential-env or --api-key-stdin");
    }
    let credential = if let Some(variable) = credential_env {
        Some(CredentialRef::Environment { variable })
    } else if api_key_stdin {
        let mut secret = String::new();
        std::io::stdin().read_to_string(&mut secret)?;
        let secret = secret.trim();
        if secret.is_empty() {
            bail!("stdin contained no API key");
        }
        store_os_secret(&id, secret)?;
        Some(CredentialRef::OsKeychain {
            account: id.clone(),
        })
    } else {
        None
    };
    let mut config = RuntimeConfig::load(root)?;
    config.executors.insert(
        id.clone(),
        ExecutorConfig {
            provider: provider.clone(),
            model: model.clone(),
            endpoint,
            credential,
        },
    );
    config.save(root)?;
    emit(
        json!({"status": "configured", "executor": id, "provider": provider, "model": model}),
        json_output,
    );
    Ok(ExitCode::SUCCESS)
}

pub fn configure_executor_list(root: &Path, json_output: bool) -> Result<ExitCode> {
    let config = RuntimeConfig::load(root)?;
    let executors = config
        .executors
        .iter()
        .map(|(id, executor)| {
            json!({
                "id": id,
                "provider": executor.provider,
                "model": executor.model,
                "default": id == &config.default_executor,
                "credential_configured": executor.credential.is_some(),
            })
        })
        .collect::<Vec<_>>();
    emit(json!({"executors": executors}), json_output);
    Ok(ExitCode::SUCCESS)
}

pub fn configure_executor_test(root: &Path, id: &str, json_output: bool) -> Result<ExitCode> {
    let config = RuntimeConfig::load(root)?;
    let executor = config
        .executors
        .get(id)
        .with_context(|| format!("executor `{id}` is not configured"))?;
    if executor.provider != "mock" {
        let key = resolve_credential(executor)?;
        let mut driver = http_executor(executor, key)?;
        driver.complete(ModelRequest::new(
            format!("configure-test-{}", ulid::Ulid::new()),
            vec![ModelMessage::user("Reply with OK.")],
        ))?;
    }
    emit(json!({"status": "ok", "executor": id}), json_output);
    Ok(ExitCode::SUCCESS)
}

pub fn configure_executor_remove(root: &Path, id: &str, json_output: bool) -> Result<ExitCode> {
    let mut config = RuntimeConfig::load(root)?;
    if id == config.default_executor {
        bail!("cannot remove default executor `{id}`; choose another default first");
    }
    config
        .executors
        .remove(id)
        .with_context(|| format!("executor `{id}` is not configured"))?;
    config.save(root)?;
    emit(json!({"status": "removed", "executor": id}), json_output);
    Ok(ExitCode::SUCCESS)
}

pub fn configure_executor_default(root: &Path, id: &str, json_output: bool) -> Result<ExitCode> {
    let mut config = RuntimeConfig::load(root)?;
    if !config.executors.contains_key(id) {
        bail!("executor `{id}` is not configured");
    }
    config.default_executor = id.to_string();
    config.save(root)?;
    emit(json!({"status": "default", "executor": id}), json_output);
    Ok(ExitCode::SUCCESS)
}

pub fn init(root: &Path, executor: &str, json_output: bool) -> Result<ExitCode> {
    if executor != "mock" {
        bail!(
            "init currently accepts only `--executor mock`; add real providers with `keel configure`"
        );
    }
    commands::init(root)?;
    commands::compile(root)?;
    commands::lock(root, false)?;
    RuntimeConfig::mock().save(root)?;
    RuntimeStore::open(&state_dir(root).join(RUNTIME_DB))?;

    emit(
        json!({
            "status": "ready",
            "workspace": root,
            "executor": executor,
        }),
        json_output,
    );
    Ok(ExitCode::SUCCESS)
}

pub fn doctor(root: &Path, json_output: bool) -> Result<ExitCode> {
    let files = WorkspaceFiles::empty(root.to_path_buf());
    let snapshot = Snapshot::load(&files.snapshot_path())?;
    let lock = Lock::load(root)?;
    if lock.snapshot_hash != snapshot.hash.to_string() {
        bail!("lock and published snapshot differ");
    }
    let config = RuntimeConfig::load(root)?;
    if !config.executors.contains_key(&config.default_executor) {
        bail!(
            "default executor `{}` is not configured",
            config.default_executor
        );
    }
    RuntimeStore::open(&state_dir(root).join(RUNTIME_DB))?;
    emit(
        json!({
            "status": "ready",
            "governed": true,
            "snapshot_hash": snapshot.hash,
            "default_executor": config.default_executor,
        }),
        json_output,
    );
    Ok(ExitCode::SUCCESS)
}

pub fn run(
    root: &Path,
    task: Option<&str>,
    resume: Option<&str>,
    requested_executor: Option<&str>,
    json_output: bool,
) -> Result<ExitCode> {
    let files = WorkspaceFiles::empty(root.to_path_buf());
    let snapshot = Snapshot::load(&files.snapshot_path())?;
    let lock = Lock::load(root)?;
    if lock.snapshot_hash != snapshot.hash.to_string() {
        bail!("lock and published snapshot differ; run `keel lock` before starting a session");
    }
    let config = RuntimeConfig::load(root)?;
    let session_id = resume
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("session-{}", ulid::Ulid::new()));
    let runtime_db = state_dir(root).join(RUNTIME_DB);
    let store = RuntimeStore::open(&runtime_db)?;
    let (task, executor_id) = if resume.is_some() {
        let metadata = store
            .session_metadata(&session_id)?
            .with_context(|| format!("session `{session_id}` has no execution metadata"))?;
        if let Some(requested) = requested_executor
            && requested != metadata.executor_id
        {
            bail!(
                "session `{session_id}` is pinned to executor `{}`; `{requested}` cannot replace it",
                metadata.executor_id
            );
        }
        (metadata.task, metadata.executor_id)
    } else {
        let task = task.expect("clap requires task").to_string();
        let executor_id = requested_executor
            .unwrap_or(&config.default_executor)
            .to_string();
        store.ensure_session(&session_id, &snapshot.hash.to_string())?;
        store.ensure_session_metadata(&session_id, &task, &executor_id)?;
        (task, executor_id)
    };
    let executor_config = config
        .executors
        .get(&executor_id)
        .with_context(|| format!("executor `{executor_id}` is not configured"))?;
    let mut host =
        RuntimeHost::from_snapshot_with_store(session_id.clone(), &snapshot, root, store)?;
    let mut capabilities = CapabilityManager::from_snapshot(root, &snapshot);
    let broker = AgentBroker::from_snapshot(&snapshot, root);
    let mut scheduler = AgentScheduler::open(&runtime_db, 4)?;
    let mut services = SessionServices {
        capabilities: &mut capabilities,
        broker: &broker,
        scheduler: &mut scheduler,
        config: &config,
    };

    if host.current_phase() != Phase::Delivery {
        if executor_config.provider == "mock" {
            let responses = Phase::all_non_terminal()
                .map(|phase| ModelResponse::text(format!("mock completed {}", phase.as_str())));
            let mut executor = MockModelExecutor::from_responses(responses);
            execute_session(&mut host, &task, &mut executor, &mut services)?;
        } else {
            let key = resolve_credential(executor_config)?;
            let mut executor = http_executor(executor_config, key)?;
            execute_session(&mut host, &task, &mut executor, &mut services)?;
        }
    }

    let completed = host.current_phase() == Phase::Delivery;
    emit(
        json!({
            "status": if completed { "completed" } else { "running" },
            "session_id": host.session_id(),
            "phase": host.current_phase().as_str(),
            "snapshot_hash": host.snapshot_hash(),
            "executor": executor_id,
        }),
        json_output,
    );
    Ok(if completed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn execute_session<E: ModelExecutor>(
    host: &mut RuntimeHost,
    task: &str,
    executor: &mut E,
    services: &mut SessionServices<'_>,
) -> Result<()> {
    let schema = json!({
        "type": "object",
        "required": ["task", "phase", "model_output"],
        "properties": {
            "task": { "type": "string" },
            "phase": { "type": "string" },
            "model_output": { "type": "string" }
        },
        "additionalProperties": false
    });
    let stages = [
        (
            Phase::Investigation,
            ArtifactKind::InvestigationReport,
            Phase::Planning,
        ),
        (
            Phase::Planning,
            ArtifactKind::SolutionContract,
            Phase::Implementation,
        ),
        (
            Phase::Implementation,
            ArtifactKind::ImplementationRecord,
            Phase::Verification,
        ),
        (
            Phase::Verification,
            ArtifactKind::EvidenceReport,
            Phase::Audit,
        ),
        (Phase::Audit, ArtifactKind::AuditReport, Phase::Resolution),
        (
            Phase::Resolution,
            ArtifactKind::ResolutionRecord,
            Phase::Acceptance,
        ),
        (
            Phase::Acceptance,
            ArtifactKind::AcceptanceRecord,
            Phase::Delivery,
        ),
    ];

    for (phase, artifact, next) in stages {
        if host.current_phase() != phase {
            continue;
        }
        let mut context = Vec::new();
        for required in host.missing_required_components() {
            if required.kind == "skill" {
                let mut request = SkillReadRequest::compact(required.id, phase.as_str());
                request.reason = required.reason;
                context.push(host.read_skill(request)?.content);
            } else {
                let mut request =
                    ComponentReadRequest::new(required.kind, required.id, phase.as_str());
                request.reason = required.reason;
                context.push(host.read_component(request)?.content);
            }
        }
        let response = execute_phase_loop(host, task, phase, executor, services, context)?;
        host.record_artifact(
            artifact,
            &json!({
                "task": task,
                "phase": phase.as_str(),
                "model_output": response.content,
            }),
            &schema,
        )?;
        host.advance_phase(next)?;
    }
    host.authorize(Operation::Delivery)?;
    host.authorize(Operation::SessionClose)?;
    Ok(())
}

fn execute_phase_loop<E: ModelExecutor>(
    host: &mut RuntimeHost,
    task: &str,
    phase: Phase,
    executor: &mut E,
    services: &mut SessionServices<'_>,
    mut context: Vec<String>,
) -> Result<ModelResponse> {
    for _ in 0..16 {
        let mut request = ModelRequest::new(
            host.session_id(),
            vec![ModelMessage::user(format!(
                "Task: {task}\nCurrent Keel phase: {}. Use Keel operations when needed, then produce the phase artifact content.",
                phase.as_str()
            ))],
        );
        request.context = context.clone();
        request.tools = tool_definitions(services.capabilities, services.broker.has_agents());
        let response = host.execute_model_turn(executor, request)?;
        if response.tool_calls.is_empty() {
            return Ok(response);
        }
        for call in response.tool_calls {
            let result = dispatch_tool_call(host, phase, services, call)?;
            context.push(format!(
                "Keel operation result: {}",
                serde_json::to_string(&result)?
            ));
        }
    }
    bail!(
        "model exceeded 16 governed operations in phase {}",
        phase.as_str()
    )
}

fn dispatch_tool_call(
    host: &mut RuntimeHost,
    phase: Phase,
    services: &mut SessionServices<'_>,
    call: ToolCall,
) -> Result<serde_json::Value> {
    match call.name.as_str() {
        "component.list" => Ok(serde_json::to_value(host.component_list())?),
        "skill.read" => {
            let id = required_argument(&call.arguments, "skill_id")?;
            let mut request = SkillReadRequest::compact(id, phase.as_str());
            request.reason = Some("model requested through Keel".to_string());
            Ok(serde_json::to_value(host.read_skill(request)?)?)
        }
        "knowledge.read" | "blueprint.read" | "component.read" => {
            let kind = if call.name == "component.read" {
                required_argument(&call.arguments, "kind")?
            } else {
                call.name.trim_end_matches(".read")
            };
            let id = required_argument(&call.arguments, "id")?;
            let mut request = ComponentReadRequest::new(kind, id, phase.as_str());
            request.reason = Some("model requested through Keel".to_string());
            Ok(serde_json::to_value(host.read_component(request)?)?)
        }
        "agent.invoke" => {
            host.authorize(Operation::AgentInvoke)?;
            let agent_id = required_argument(&call.arguments, "agent_id")?;
            let input = required_argument(&call.arguments, "input")?;
            let executor_id = services
                .broker
                .executor_for(agent_id)
                .with_context(|| format!("agent `{agent_id}` is not declared"))?;
            let child_config = services
                .config
                .executors
                .get(executor_id)
                .with_context(|| {
                    format!("executor `{executor_id}` for agent `{agent_id}` is not configured")
                })?;
            let result = if child_config.provider == "mock" {
                let mut child = MockModelExecutor::with_response(ModelResponse::text(format!(
                    "mock agent {agent_id} completed"
                )));
                services.broker.invoke(
                    host.session_id(),
                    agent_id,
                    input,
                    services.scheduler,
                    &mut child,
                )?
            } else {
                let key = resolve_credential(child_config)?;
                let mut child = http_executor(child_config, key)?;
                services.broker.invoke(
                    host.session_id(),
                    agent_id,
                    input,
                    services.scheduler,
                    &mut child,
                )?
            };
            Ok(serde_json::to_value(result)?)
        }
        capability => {
            Ok(serde_json::to_value(services.capabilities.execute(
                &CapabilityRequest::new(capability, call.arguments),
            )?)?)
        }
    }
}

fn required_argument<'a>(arguments: &'a serde_json::Value, name: &str) -> Result<&'a str> {
    arguments
        .get(name)
        .and_then(serde_json::Value::as_str)
        .with_context(|| format!("tool argument `{name}` must be a string"))
}

fn tool_definitions(
    capabilities: &CapabilityManager,
    agent_invocation_available: bool,
) -> Vec<ToolDefinition> {
    let object = || json!({"type": "object", "additionalProperties": true});
    let mut tools = vec![
        ToolDefinition {
            name: "component.list".to_string(),
            description: "List components available through Keel".to_string(),
            input_schema: json!({"type": "object", "additionalProperties": false}),
        },
        ToolDefinition {
            name: "skill.read".to_string(),
            description: "Read a Keel-owned skill and record its receipt".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["skill_id"],
                "properties": {"skill_id": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "component.read".to_string(),
            description: "Read a Keel-owned component".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["kind", "id"],
                "properties": {"kind": {"type": "string"}, "id": {"type": "string"}},
                "additionalProperties": false
            }),
        },
    ];
    if agent_invocation_available {
        tools.push(ToolDefinition {
            name: "agent.invoke".to_string(),
            description: "Invoke a declared logical agent through Keel".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["agent_id", "input"],
                "properties": {
                    "agent_id": {"type": "string"},
                    "input": {"type": "string"}
                },
                "additionalProperties": false
            }),
        });
    }
    tools.extend(capabilities.grants().map(|name| ToolDefinition {
        name: name.to_string(),
        description: format!("Governed capability `{name}`"),
        input_schema: object(),
    }));
    tools
}

fn http_executor(config: &ExecutorConfig, api_key: String) -> Result<HttpModelExecutor> {
    let provider = match config.provider.as_str() {
        "anthropic" => HttpProvider::Anthropic,
        "openai" => HttpProvider::OpenAi,
        other => bail!("unsupported HTTP provider `{other}`"),
    };
    let endpoint = config.endpoint.clone().unwrap_or_else(|| match provider {
        HttpProvider::Anthropic => "https://api.anthropic.com/v1/messages".to_string(),
        HttpProvider::OpenAi => "https://api.openai.com/v1/responses".to_string(),
    });
    Ok(HttpModelExecutor::new(
        provider,
        &config.model,
        endpoint,
        api_key,
    )?)
}

fn resolve_credential(config: &ExecutorConfig) -> Result<String> {
    match config
        .credential
        .as_ref()
        .context("executor has no credential reference")?
    {
        CredentialRef::Environment { variable } => std::env::var(variable)
            .with_context(|| format!("credential environment variable `{variable}` is not set")),
        CredentialRef::OsKeychain { account } => load_os_secret(account),
    }
}

fn store_os_secret(account: &str, secret: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("security")
            .args([
                "add-generic-password",
                "-U",
                "-s",
                "dev.keel.executor",
                "-a",
                account,
                "-w",
                secret,
            ])
            .status()?;
        if !status.success() {
            bail!("could not store credential in macOS Keychain");
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        use std::io::Write;
        let mut child = Command::new("secret-tool")
            .args([
                "store",
                "--label",
                "Keel executor",
                "service",
                "keel",
                "executor",
                account,
            ])
            .stdin(Stdio::piped())
            .spawn()
            .context("secret-tool is required for Secret Service storage")?;
        child
            .stdin
            .take()
            .context("secret-tool stdin")?
            .write_all(secret.as_bytes())?;
        if !child.wait()?.success() {
            bail!("could not store credential in Secret Service");
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        bail!("OS keychain storage is supported only on macOS and Linux")
    }
}

fn load_os_secret(account: &str) -> Result<String> {
    #[cfg(target_os = "macos")]
    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "dev.keel.executor",
            "-a",
            account,
            "-w",
        ])
        .stderr(Stdio::null())
        .output()?;
    #[cfg(target_os = "linux")]
    let output = Command::new("secret-tool")
        .args(["lookup", "service", "keel", "executor", account])
        .stderr(Stdio::null())
        .output()
        .context("secret-tool is required for Secret Service lookup")?;
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    bail!("OS keychain lookup is supported only on macOS and Linux");
    if !output.status.success() {
        bail!("credential `{account}` was not found in the OS keychain");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn state_dir(root: &Path) -> std::path::PathBuf {
    root.join(keel_engine::workspace::STATE_DIR)
}

fn emit(value: serde_json::Value, json_output: bool) {
    if json_output {
        println!(
            "{}",
            serde_json::to_string(&value).expect("JSON value serializes")
        );
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("JSON value serializes")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecutorConfig, RuntimeConfig, SessionServices, dispatch_tool_call};
    use keel_engine::snapshot::{CompiledAgent, Snapshot};
    use keel_runtime::{
        AgentBroker, AgentScheduler, CapabilityManager, Phase, RuntimeHost, ToolCall,
    };
    use std::collections::BTreeMap;

    #[test]
    fn agent_invoke_uses_the_executor_selected_by_the_broker() {
        let root = tempfile::tempdir().unwrap();
        let agents = BTreeMap::from([(
            "reviewer".to_string(),
            CompiledAgent {
                id: "reviewer".to_string(),
                role: "review".to_string(),
                executor: "codex".to_string(),
                objective: Some("Review the result".to_string()),
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
        let mut capabilities = CapabilityManager::new(root.path());
        let mut host = RuntimeHost::new("session", snapshot.hash.to_string());
        let config = RuntimeConfig {
            default_executor: "parent".to_string(),
            executors: BTreeMap::from([(
                "codex".to_string(),
                ExecutorConfig {
                    provider: "mock".to_string(),
                    model: "mock-codex".to_string(),
                    endpoint: None,
                    credential: None,
                },
            )]),
        };

        let mut services = SessionServices {
            capabilities: &mut capabilities,
            broker: &broker,
            scheduler: &mut scheduler,
            config: &config,
        };
        let result = dispatch_tool_call(
            &mut host,
            Phase::Investigation,
            &mut services,
            ToolCall {
                name: "agent.invoke".to_string(),
                arguments: serde_json::json!({"agent_id": "reviewer", "input": "inspect"}),
            },
        )
        .unwrap();

        assert_eq!(result["executor_id"], "codex");
        assert_eq!(result["agent_id"], "reviewer");
    }
}
