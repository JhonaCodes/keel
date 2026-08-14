// SPDX-License-Identifier: Apache-2.0
//! `keel <cli>` — assembling the governed environment and launching the child.
//!
//! Order matters and is the whole point (spec section 5.3): the snapshot is
//! loaded and pinned FIRST, the containment (shims → broker) is built SECOND,
//! and only then does the client CLI get to exist — inside it.

use crate::broker::{Broker, spawn as spawn_broker};
use crate::{pty, sandbox, shims, supervisor};
use anyhow::{Context, Result, bail};
use keel_engine::adapter::AdapterManifest;
use keel_engine::ledger::Ledger;
use keel_engine::snapshot::Snapshot;
use keel_engine::workspace::WorkspaceFiles;
use keel_runtime::RuntimeStore;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::ExitCode;

const RUNTIME_DB: &str = "runtime.sqlite";

pub struct LaunchOptions {
    /// Launch adapter id (`claude`, `codex`, `opencode`, `generic`).
    pub client: String,
    /// Explicit workspace; otherwise `KEEL_WORKSPACE`, then walk-up from cwd.
    pub workspace: Option<PathBuf>,
    /// Extra argv appended to the adapter's base command (for `generic` it IS
    /// the command).
    pub cmd: Vec<String>,
    /// Initial task, appended as a positional argument when present.
    pub task: Option<String>,
    /// Resume a keel session identity; a fresh ulid otherwise.
    pub session: Option<String>,
    /// Requested containment level. `Full` (default) uses the OS sandbox when
    /// available; `Shims` skips it deliberately (the banner says so).
    pub containment: ContainmentMode,
    /// When true, the supervisor does not surface cognitive-direction
    /// suggestions (P3). Enforcement is unaffected.
    pub no_suggest: bool,
}

/// What the operator asked for. The EFFECTIVE level (see `sandbox::Level`) can
/// be lower — always with a banner, never silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContainmentMode {
    /// Shims + OS sandbox when a provider is available.
    #[default]
    Full,
    /// Shims only — explicit opt-out of the hard ring.
    Shims,
}

pub fn launch(opts: LaunchOptions) -> Result<ExitCode> {
    let root = resolve_workspace(opts.workspace.clone())?;
    // Secrets: a gitignored `<workspace>/.env` is loaded into keel's environment
    // so `${VAR}` in executor/provider configs resolves from it. The child
    // inherits this env (pty::run does not env_clear), so the launched client
    // and its `keel mcp` subprocess see it too. A shell export still wins.
    crate::dotenv::load_workspace_env(&root);
    let files = WorkspaceFiles::empty(root.clone());
    let snapshot = Snapshot::load(&files.snapshot_path())
        .context("no published snapshot — run `keel compile` first")?;

    let manifest = AdapterManifest::for_client(&opts.client).with_context(|| {
        format!(
            "unknown client `{}` — use `keel launch --client generic -- <cmd>`",
            opts.client
        )
    })?;

    // Invariant 8, out loud: a block the containment cannot honor is reported
    // BEFORE the session starts, never silently promised.
    for violation in keel_engine::adapter::preflight(&snapshot, &manifest) {
        eprintln!("[keel] preflight: {}", violation.reason);
    }

    let mut argv = manifest.command.clone();
    argv.extend(opts.cmd.iter().cloned());
    if let Some(task) = &opts.task {
        argv.push(task.clone());
    }
    if argv.is_empty() {
        bail!("client `generic` needs an explicit command: keel launch --client generic -- <cmd>");
    }

    let session_id = opts
        .session
        .clone()
        .unwrap_or_else(|| format!("session-{}", ulid::Ulid::new()));

    // Session state dir: sealed (0700), everything ephemeral lives here; the
    // ledger and the runtime store live OUTSIDE it and survive the session.
    let state_dir = files.state_dir();
    let host_dir = state_dir.join("host").join(&session_id);
    std::fs::create_dir_all(&host_dir)?;
    std::fs::set_permissions(&host_dir, std::fs::Permissions::from_mode(0o700))?;

    let store = RuntimeStore::open(&state_dir.join(RUNTIME_DB))?;
    store.ensure_session(&session_id, &snapshot.hash.to_string())?;

    // The socket lives OUTSIDE the workspace, in a short temp path: a UNIX
    // socket path must fit in SUN_LEN (~104 bytes on macOS), which a nested
    // workspace/.keel-state/host/<ulid>/ path easily overflows. It is not
    // security-bearing (it only evaluates; identity is baked into the shims),
    // so a short unique name is enough. Cleaned up on teardown.
    let socket_path = std::env::temp_dir().join(format!("keel-{}.sock", short_id(&session_id)));
    let ledger = Ledger::open(&files.ledger_path())?;
    // Capture what the announce, sandbox, and convergence need before the
    // snapshot moves into the broker: the containment, the governed skill ids
    // (so keel can name them to the model up front, not just say "go list
    // them"), and the compiled components (H-011: resolves configured
    // `MCPProvider`s for `wire_convergence`).
    let snapshot_containment = snapshot.containment.clone();
    let skill_ids: Vec<String> = snapshot.skills.keys().cloned().collect();
    let components = snapshot.components.clone();
    let broker = Broker::new(snapshot, ledger, root.clone(), session_id.clone());
    let (join, handle) = spawn_broker(broker, &socket_path)?;

    let shim_bin = shims::shim_binary()?;
    let shim_dir = shims::generate(&host_dir, &shim_bin, &socket_path, &manifest.shim_commands)?;

    // The fabricated environment: interposition first on PATH; KEEL_* vars are
    // INFORMATIVE (session identity travels baked inside the shims, never
    // through env the child could rewrite).
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    env.insert(
        "PATH".into(),
        format!("{}:{}", shim_dir.display(), inherited_path),
    );
    env.insert("KEEL_SESSION".into(), session_id.clone());
    env.insert("KEEL_WORKSPACE".into(), root.display().to_string());
    // Which client is being governed, so workspace tools (e.g. keel-catalog)
    // can deliver agents the same way keel's native delivery does: a client with
    // in-session subagents (Claude Code) gets the Task-subagent path for agents
    // that declare `nativeSubagent`, not the external `keel.agent.invoke` CLI.
    env.insert("KEEL_CLIENT".into(), opts.client.clone());

    // Convergence plane (P2): wire keel's MCP endpoint into the child so the
    // model discovers/loads its governed skills through keel, and announce it.
    // Ephemeral, per-session config; if the child ignores or deletes it, the
    // hard rings are unaffected (P1 never depends on P2).
    let argv = wire_convergence(
        argv,
        &mut env,
        &manifest,
        ConvergenceContext {
            host_dir: &host_dir,
            root: &root,
            session_id: &session_id,
            skill_ids: &skill_ids,
            components: &components,
        },
    )?;

    // The hard ring: wrap argv in the OS sandbox when the snapshot declares a
    // containment AND a provider can honor it. Any downgrade is announced.
    // Even with NO declared Containment, a wired hook needs the sandbox so it
    // can protect keel's control surface (.keel-state) — synthesize an empty
    // containment in that case (the profile always adds the .keel-state deny).
    let effective_containment = snapshot_containment.clone().or_else(|| {
        manifest
            .hook
            .is_some()
            .then(|| keel_engine::snapshot::CompiledContainment {
                deny_unlink: Vec::new(),
                deny_write_outside: false,
                deny_network: false,
            })
    });
    let (argv, level) = apply_sandbox(argv, &effective_containment, &root, opts.containment);

    eprintln!(
        "[keel] session {session_id} — containment: {} ({}) — workspace {}",
        level_label(level),
        manifest.id,
        root.display()
    );

    // Cognitive direction (P3): a supervisor watches the live ledger and
    // surfaces suggestions (oscillation) to the operator's transcript — never
    // into the model's input stream (that would interfere with its reasoning).
    let supervisor_shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let supervisor = (!opts.no_suggest).then(|| {
        supervisor::spawn(
            files.ledger_path(),
            session_id.clone(),
            supervisor_shutdown.clone(),
        )
    });

    let cwd = std::env::current_dir()?;
    let code = pty::run(&argv, &env, &cwd);

    // Teardown: supervisor + broker down, ephemeral dir + socket gone; evidence
    // stays.
    supervisor_shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(supervisor) = supervisor {
        let _ = supervisor.join();
    }
    handle.stop();
    let _ = join.join();
    let _ = std::fs::remove_dir_all(&host_dir);
    let _ = std::fs::remove_file(&socket_path);

    let code = code?;
    Ok(if code == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(code.clamp(0, 255) as u8)
    })
}

/// Read-only launch context `wire_convergence` needs, captured before the
/// snapshot moves into the broker (H-011 added `components` alongside the
/// pre-existing `skill_ids` — grouped here rather than as more positional
/// arguments).
struct ConvergenceContext<'a> {
    host_dir: &'a std::path::Path,
    root: &'a std::path::Path,
    session_id: &'a str,
    skill_ids: &'a [String],
    components: &'a BTreeMap<String, keel_engine::snapshot::CompiledComponent>,
}

/// Wires keel's MCP endpoint into the child per the adapter manifest and
/// returns the augmented argv. Writes an ephemeral per-session MCP config
/// under `ctx.host_dir`; appends the client's config flag and the "you are
/// governed by keel" announcement. A client with no known wiring (generic)
/// is returned unchanged — convergence is opt-in there; the hard rings hold
/// regardless.
fn wire_convergence(
    mut argv: Vec<String>,
    env: &mut BTreeMap<String, String>,
    manifest: &AdapterManifest,
    ctx: ConvergenceContext<'_>,
) -> Result<Vec<String>> {
    let ConvergenceContext {
        host_dir,
        root,
        session_id,
        skill_ids,
        components,
    } = ctx;
    use keel_engine::adapter::{Announce, HookMethod, McpMethod};

    if manifest.mcp.is_none() && manifest.hook.is_none() {
        return Ok(argv);
    }
    let keel_bin =
        std::env::current_exe().context("convergence: cannot resolve the keel binary")?;
    let keel_bin = keel_bin.display().to_string();
    let root_str = root.display().to_string();

    if let Some(mcp) = &manifest.mcp {
        // Every explicitly enabled `MCPProvider` (H-011) wires in ALONGSIDE
        // keel's own entry. Providers are fail-closed: `config.enabled: true`
        // is required to stay active. Optional/local providers remain in the
        // governed workspace without slowing or hanging every launched client.
        // `wire_convergence`
        // runs at launch time, before any prompt/tool "moment" text exists to
        // route a `match` block against (D-014), so provider selection cannot
        // be prompt-conditional here.
        let mut servers = vec![keel_runtime::McpServerSpec {
            name: "keel".to_string(),
            command: vec![
                keel_bin.clone(),
                "mcp".to_string(),
                "--workspace".to_string(),
                root_str.clone(),
                "--session".to_string(),
                session_id.to_string(),
            ],
            env: Vec::new(),
        }];
        servers.extend(
            keel_runtime::compiled_mcp_providers(components)
                .context("convergence: resolving configured MCP providers")?,
        );

        match &mcp.method {
            McpMethod::ConfigFileFlag { flag } => {
                // Claude-style: a JSON file of MCP servers.
                let mut mcp_servers = serde_json::Map::new();
                for server in &servers {
                    let mut entry = serde_json::json!({
                        "command": server.command[0],
                        "args": server.command[1..],
                    });
                    if !server.env.is_empty() {
                        entry["env"] = server
                            .env
                            .iter()
                            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                            .collect();
                    }
                    mcp_servers.insert(server.name.clone(), entry);
                }
                let config = serde_json::json!({ "mcpServers": mcp_servers });
                let path = host_dir.join("mcp.json");
                std::fs::write(&path, serde_json::to_string_pretty(&config)?)?;
                argv.push(flag.clone());
                argv.push(path.display().to_string());
            }
            McpMethod::ConfigOverrideFlag { flag } => {
                // Codex-style: dotted TOML overrides, one triple per server.
                for server in &servers {
                    let name = &server.name;
                    let args_toml = format!(
                        "[{}]",
                        server.command[1..]
                            .iter()
                            .map(|a| toml_string(a))
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                    argv.push(flag.clone());
                    argv.push(format!(
                        "mcp_servers.{name}.command={}",
                        toml_string(&server.command[0])
                    ));
                    argv.push(flag.clone());
                    argv.push(format!("mcp_servers.{name}.args={args_toml}"));
                    if !server.env.is_empty() {
                        let env_toml = format!(
                            "{{{}}}",
                            server
                                .env
                                .iter()
                                .map(|(k, v)| format!("{k}={}", toml_string(v)))
                                .collect::<Vec<_>>()
                                .join(",")
                        );
                        argv.push(flag.clone());
                        argv.push(format!("mcp_servers.{name}.env={env_toml}"));
                    }
                }
            }
            McpMethod::EnvConfigVar { var } => {
                // OpenCode-style: runtime config JSON from an environment variable.
                let mut mcp = serde_json::Map::new();
                for server in &servers {
                    let mut entry = serde_json::json!({
                        "type": "local",
                        "command": server.command,
                        "enabled": true,
                    });
                    if !server.env.is_empty() {
                        entry["environment"] = server
                            .env
                            .iter()
                            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                            .collect();
                    }
                    mcp.insert(server.name.clone(), entry);
                }
                let config = serde_json::json!({
                    "$schema": "https://opencode.ai/config.json",
                    "mcp": mcp,
                });
                env.insert(var.clone(), serde_json::to_string(&config)?);
            }
        }

        let catalog = if skill_ids.is_empty() {
            String::new()
        } else {
            format!(
                " Governed skills you can load right now: {}.",
                skill_ids.join(", ")
            )
        };
        let notice = format!(
            "You are running under keel, a local governance runtime. Your skills and agents \
         are provided BY keel through MCP tools (keel.skills.list, keel.skills.load, \
         keel.rules.query, keel.agent.invoke).{catalog} BEFORE starting a task that one \
         of them covers, load it with keel.skills.load and FOLLOW it — do not use your \
         own defaults when keel offers a skill. Some actions are contained by keel and \
         will be refused; read the block message and adjust."
        );
        match &mcp.announce {
            Announce::SystemPromptFlag { flag } => {
                argv.push(flag.clone());
                argv.push(notice.to_string());
            }
            Announce::PtyLine => {
                // No client flag: the notice is printed to the session so both the
                // operator and the model see it at start.
                eprintln!("[keel] {notice}");
            }
        }
    } // end MCP wiring

    // Hook bridge: install keel's `keel gate` as a PreToolUse hook, so keel sees
    // the client's INTERNAL tool calls (Write/Edit) too. Ephemeral per-session
    // settings file; the OS sandbox denies the child writing keel's state
    // (incl. this file), so the model cannot remove the hook.
    if let Some(hook) = &manifest.hook {
        match &hook.method {
            HookMethod::SettingsFileFlag { flag } => {
                let command = format!(
                    "{keel_bin} gate --client {} --workspace {root_str} --session {session_id}",
                    hook.dialect
                );
                let settings = serde_json::json!({
                    "hooks": {
                        // Catch-all matcher (""): keel SEES every tool the model is
                        // about to use — Bash/Edit/Write/WebFetch map to their
                        // governance events; anything else (a native MCP tool, a
                        // read, a search) becomes `tool.requested` so keel can
                        // DELIVER relevant context at that moment (D-016). It only
                        // BLOCKS the specific pre-action events; the rest is
                        // observe + opportune delivery, never a block.
                        "PreToolUse": [{
                            "matcher": "",
                            "hooks": [{ "type": "command", "command": command }]
                        }],
                        // PostToolUse: AFTER a tool finished, catch-all too. This is
                        // the only moment a Bash test-runner's real exit code and
                        // output exist — it is how RED/GREEN evidence
                        // (`test.completed`, gate.rs::is_test_runner) enters the
                        // ledger. Without this, `evidence.recorded` preconditions
                        // (e.g. require-red-before-write) can NEVER be satisfied in
                        // a governed Claude session — not stricter, impossible.
                        "PostToolUse": [{
                            "matcher": "",
                            "hooks": [{ "type": "command", "command": command }]
                        }],
                        // UserPromptSubmit: keel enriches the prompt with the
                        // output of `prompt.submitted` rules (D-013), so the model
                        // receives the task already deserialized.
                        "UserPromptSubmit": [{
                            "hooks": [{ "type": "command", "command": command }]
                        }],
                        // SessionStart: deliver base context/rules up front (D-016).
                        "SessionStart": [{
                            "hooks": [{ "type": "command", "command": command }]
                        }],
                        "Stop": [{
                            "hooks": [{ "type": "command", "command": command }]
                        }]
                    }
                });
                let path = host_dir.join("settings.json");
                std::fs::write(&path, serde_json::to_string_pretty(&settings)?)?;
                argv.push(flag.clone());
                argv.push(path.display().to_string());
            }
            HookMethod::ConfigOverrideFlags {
                flag,
                trust_bypass_flag,
            } => {
                let script = write_codex_gate_script(host_dir, &keel_bin, &root_str, session_id)?;
                let hook_command = shell_quote(&script.display().to_string());
                let hook = format!(
                    "{{hooks=[{{type=\"command\",command={},statusMessage=\"Keel gate\"}}]}}",
                    toml_string(&hook_command)
                );
                argv.push(flag.clone());
                argv.push("features.hooks=true".into());
                for event in [
                    "SessionStart",
                    "UserPromptSubmit",
                    "PreToolUse",
                    "PostToolUse",
                    "Stop",
                ] {
                    argv.push(flag.clone());
                    argv.push(format!("hooks.{event}=[{hook}]"));
                }
                if let Some(trust_bypass_flag) = trust_bypass_flag {
                    argv.push(trust_bypass_flag.clone());
                }
            }
            HookMethod::ConfigDirEnv { var } => {
                let config_dir = host_dir.join("opencode");
                let plugins_dir = config_dir.join("plugins");
                std::fs::create_dir_all(&plugins_dir)?;
                let plugin = opencode_gate_plugin(&keel_bin, &root_str, session_id);
                std::fs::write(plugins_dir.join("keel-gate.js"), plugin)?;
                env.insert(var.clone(), config_dir.display().to_string());
            }
        }
    }

    Ok(argv)
}

/// Wraps `argv` in the OS sandbox when the snapshot declares a containment
/// and a provider can honor it. Returns the (possibly wrapped) argv and the
/// EFFECTIVE level. Every downgrade from what was requested is announced —
/// a security boundary must never weaken silently.
fn apply_sandbox(
    argv: Vec<String>,
    containment: &Option<keel_engine::snapshot::CompiledContainment>,
    workspace: &std::path::Path,
    mode: ContainmentMode,
) -> (Vec<String>, sandbox::Level) {
    let Some(containment) = containment else {
        // Nothing to enforce at the kernel: shims are the only ring by design.
        return (argv, sandbox::Level::Shims);
    };
    if mode == ContainmentMode::Shims {
        eprintln!(
            "[keel] containment: shims-only by request — the OS sandbox is OFF; \
             an absolute-path command can bypass interposition"
        );
        return (argv, sandbox::Level::Shims);
    }
    match sandbox::provider() {
        Some(provider) if provider.available() => (
            provider.wrap(&argv, containment, workspace),
            sandbox::Level::Full,
        ),
        Some(provider) => {
            eprintln!(
                "[keel] containment: DEGRADED to shims — the `{}` sandbox is not \
                 available here; an absolute-path command can bypass interposition",
                provider.name()
            );
            (argv, sandbox::Level::Shims)
        }
        None => {
            eprintln!(
                "[keel] containment: DEGRADED to shims — no OS sandbox provider on \
                 this platform yet; an absolute-path command can bypass interposition"
            );
            (argv, sandbox::Level::Shims)
        }
    }
}

fn level_label(level: sandbox::Level) -> &'static str {
    match level {
        sandbox::Level::Full => "shims + os-sandbox",
        sandbox::Level::Shims => "shims",
    }
}

/// The tail of the session ulid — enough to keep the socket name unique and
/// short (SUN_LEN budget), without exposing the full session identity.
fn short_id(session_id: &str) -> String {
    let tail: String = session_id.chars().rev().take(12).collect();
    tail.chars().rev().collect()
}

fn write_codex_gate_script(
    host_dir: &std::path::Path,
    keel_bin: &str,
    root: &str,
    session_id: &str,
) -> Result<std::path::PathBuf> {
    let path = host_dir.join("codex-gate.sh");
    let script = format!(
        "#!/bin/sh\nexec {} gate --client codex --workspace {} --session {}\n",
        shell_quote(keel_bin),
        shell_quote(root),
        shell_quote(session_id),
    );
    std::fs::write(&path, script)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

fn toml_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn opencode_gate_plugin(keel_bin: &str, root: &str, session_id: &str) -> String {
    let keel_bin = serde_json::to_string(keel_bin).expect("string serializes");
    let root = serde_json::to_string(root).expect("string serializes");
    let session_id = serde_json::to_string(session_id).expect("string serializes");
    format!(
        r#"import {{ spawnSync }} from "node:child_process";

const KEEL_BIN = {keel_bin};
const KEEL_WORKSPACE = {root};
const KEEL_SESSION = {session_id};

function str(value) {{
  return typeof value === "string" && value.length > 0 ? value : undefined;
}}

function eventForTool(input, output) {{
  const tool = input?.tool || "";
  const args = output?.args || {{}};
  const event = {{
    kind: "tool.requested",
    session_id: KEEL_SESSION,
    content: tool,
    env: {{}},
    files: [],
    loaded_skills: [],
    recorded_evidence: [],
  }};

  if (tool === "bash") {{
    event.kind = "command.requested";
    event.command = str(args.command);
    return event;
  }}

  if (tool === "edit" || tool === "write") {{
    event.kind = "file.edited";
    event.file = str(args.filePath) || str(args.path);
    event.content = str(args.content) || str(args.newString) || str(args.new_string);
    return event;
  }}

  if (tool === "apply_patch") {{
    event.kind = "file.edited";
    event.content = str(args.patchText) || str(args.patch);
    return event;
  }}

  if (tool === "webfetch") {{
    event.kind = "command.requested";
    event.command = str(args.url);
    return event;
  }}

  event.content = `${{tool}} ${{JSON.stringify(args).slice(0, 500)}}`;
  return event;
}}

function runGate(event) {{
  const child = spawnSync(
    KEEL_BIN,
    ["gate", "--client", "native", "--workspace", KEEL_WORKSPACE, "--session", KEEL_SESSION],
    {{
      input: JSON.stringify(event),
      encoding: "utf8",
    }},
  );
  if (child.stderr) process.stderr.write(child.stderr);
  if (child.stdout) process.stderr.write(child.stdout);
  if (child.status === 2) {{
    throw new Error("keel blocked this OpenCode tool call");
  }}
  if (child.status !== 0) {{
    process.stderr.write(`[keel] opencode hook failed with exit ${{child.status}}\n`);
  }}
}}

export const KeelGate = async () => {{
  return {{
    "tool.execute.before": async (input, output) => {{
      runGate(eventForTool(input, output));
    }},
  }};
}};
"#
    )
}

/// Workspace resolution: `--workspace` > `KEEL_WORKSPACE` > walk-up from cwd >
/// the operator's registered default (`~/.keel/config.json`, set by `keel
/// init`/`keel use`). Only then an error — so `keel <cli>` works from anywhere
/// after an init.
fn resolve_workspace(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Ok(env_ws) = std::env::var("KEEL_WORKSPACE") {
        return Ok(PathBuf::from(env_ws));
    }
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("workspace.yaml").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    if let Some(default) = crate::config::default_workspace() {
        return Ok(default);
    }
    bail!(
        "no keel workspace found — pass --workspace, set KEEL_WORKSPACE, run inside \
         a workspace, or register one with `keel init` / `keel use <path>`"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use keel_engine::snapshot::{CompiledComponent, CompiledMatch};

    fn no_providers() -> BTreeMap<String, CompiledComponent> {
        BTreeMap::new()
    }

    /// A `kind: MCPProvider` component the way `compile.rs` produces one,
    /// keyed `mcp-provider:<id>` (H-011).
    fn linear_provider() -> BTreeMap<String, CompiledComponent> {
        let mut components = BTreeMap::new();
        components.insert(
            "mcp-provider:linear".to_string(),
            CompiledComponent {
                kind: "mcp-provider".into(),
                id: "linear".into(),
                version: "0".into(),
                description: None,
                match_: CompiledMatch::default(),
                content: None,
                inline: None,
                requirements: vec![],
                capabilities: vec![],
                config: Some(serde_json::json!({
                    "enabled": true,
                    "command": ["sh", "fake-linear-mcp.sh"],
                    "env": { "LINEAR_API_KEY": "${KEEL_TEST_LINEAR_KEY}" }
                })),
            },
        );
        components
    }

    fn disabled_linear_provider() -> BTreeMap<String, CompiledComponent> {
        let mut components = linear_provider();
        components
            .get_mut("mcp-provider:linear")
            .expect("linear provider")
            .config = Some(serde_json::json!({
            "enabled": false,
            "command": ["sh", "fake-linear-mcp.sh"],
            "env": { "LINEAR_API_KEY": "${KEEL_TEST_LINEAR_KEY}" }
        }));
        components
    }

    #[test]
    fn opencode_convergence_writes_mcp_config_to_env() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let host_dir = root.join("host");
        std::fs::create_dir_all(&host_dir).unwrap();

        let manifest = AdapterManifest::for_client("opencode").unwrap();
        let mut env = BTreeMap::new();
        let argv = wire_convergence(
            manifest.command.clone(),
            &mut env,
            &manifest,
            ConvergenceContext {
                host_dir: &host_dir,
                root,
                session_id: "session-test",
                skill_ids: &["keel_verification".into()],
                components: &no_providers(),
            },
        )
        .unwrap();

        assert_eq!(argv, vec!["opencode"]);
        let config = env
            .get("OPENCODE_CONFIG_CONTENT")
            .expect("opencode config env is written");
        let config: serde_json::Value = serde_json::from_str(config).unwrap();
        assert_eq!(config["mcp"]["keel"]["type"], "local");
        assert_eq!(config["mcp"]["keel"]["enabled"], true);
        assert_eq!(config["mcp"]["keel"]["command"][1], "mcp");
        assert_eq!(
            config["mcp"]["keel"]["command"][3],
            root.display().to_string()
        );
        assert_eq!(config["mcp"]["keel"]["command"][5], "session-test");

        let config_dir = env
            .get("OPENCODE_CONFIG_DIR")
            .expect("opencode config dir env is written");
        let plugin = std::fs::read_to_string(
            std::path::Path::new(config_dir)
                .join("plugins")
                .join("keel-gate.js"),
        )
        .unwrap();
        assert!(plugin.contains("\"tool.execute.before\""));
        assert!(plugin.contains("\"gate\", \"--client\", \"native\""));
    }

    #[test]
    fn codex_convergence_writes_mcp_and_hooks_to_config_overrides() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let host_dir = root.join("host");
        std::fs::create_dir_all(&host_dir).unwrap();

        let manifest = AdapterManifest::for_client("codex").unwrap();
        let mut env = BTreeMap::new();
        let argv = wire_convergence(
            manifest.command.clone(),
            &mut env,
            &manifest,
            ConvergenceContext {
                host_dir: &host_dir,
                root,
                session_id: "session-test",
                skill_ids: &["keel_verification".into()],
                components: &no_providers(),
            },
        )
        .unwrap();

        assert!(
            argv.iter()
                .any(|a| a.starts_with("mcp_servers.keel.command="))
        );
        assert!(argv.iter().any(|a| a == "features.hooks=true"));
        assert!(argv.iter().any(|a| a.starts_with("hooks.PreToolUse=[")));
        assert!(argv.iter().any(|a| a.starts_with("hooks.Stop=[")));
        assert!(argv.contains(&"--dangerously-bypass-hook-trust".to_string()));
        let script = host_dir.join("codex-gate.sh");
        assert!(script.exists());
        let script = std::fs::read_to_string(script).unwrap();
        assert!(script.contains("gate --client codex"));
    }

    #[test]
    fn claude_convergence_writes_a_posttooluse_hook_alongside_pretooluse() {
        // Regression for a live bug: RED/GREEN evidence capture (test.completed)
        // is synthesized on PostToolUse for a Bash test-runner (gate.rs). Without
        // this hook registered, a governed `keel claude` session never calls
        // `keel gate` after a test finishes, so `require-red-before-write` can
        // never see the RED evidence and blocks every production-file edit
        // forever — the TDD gate becomes impossible to satisfy, not just
        // stricter. PreToolUse alone (blocking) is not enough; delivery of
        // test outcomes requires the AFTER-the-fact hook too.
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let host_dir = root.join("host");
        std::fs::create_dir_all(&host_dir).unwrap();

        let manifest = AdapterManifest::for_client("claude").unwrap();
        let mut env = BTreeMap::new();
        let argv = wire_convergence(
            manifest.command.clone(),
            &mut env,
            &manifest,
            ConvergenceContext {
                host_dir: &host_dir,
                root,
                session_id: "session-test",
                skill_ids: &["keel_verification".into()],
                components: &no_providers(),
            },
        )
        .unwrap();

        let settings_path = argv
            .iter()
            .find(|a| a.ends_with("settings.json"))
            .expect("a --settings-style flag value pointing at the written file");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path).unwrap()).unwrap();
        let hooks = &settings["hooks"];
        for event in ["PreToolUse", "PostToolUse", "SessionStart", "Stop"] {
            assert!(
                hooks[event].is_array(),
                "expected hooks.{event} to be registered for Claude, got: {hooks}"
            );
        }
    }

    // H-011: a workspace with a configured `MCPProvider` (e.g. Linear) wires
    // it ALONGSIDE keel's own entry, into every client — not instead of it,
    // and without installing anything per client.
    #[test]
    fn claude_convergence_wires_a_configured_mcp_provider_alongside_keel() {
        // SAFETY: single-threaded test-only variable name.
        unsafe {
            std::env::set_var("KEEL_TEST_LINEAR_KEY", "sk-linear-test");
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let host_dir = root.join("host");
        std::fs::create_dir_all(&host_dir).unwrap();

        let manifest = AdapterManifest::for_client("claude").unwrap();
        let mut env = BTreeMap::new();
        let argv = wire_convergence(
            manifest.command.clone(),
            &mut env,
            &manifest,
            ConvergenceContext {
                host_dir: &host_dir,
                root,
                session_id: "session-test",
                skill_ids: &["keel_verification".into()],
                components: &linear_provider(),
            },
        )
        .unwrap();

        let mcp_path = argv
            .iter()
            .find(|a| a.ends_with("mcp.json"))
            .expect("a --mcp-config-style flag value pointing at the written file");
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(mcp_path).unwrap()).unwrap();
        assert!(
            config["mcpServers"]["keel"]["command"].is_string(),
            "keel's own entry must still be present: {config}"
        );
        assert_eq!(config["mcpServers"]["linear"]["command"], "sh");
        assert_eq!(
            config["mcpServers"]["linear"]["args"][0],
            "fake-linear-mcp.sh"
        );
        assert_eq!(
            config["mcpServers"]["linear"]["env"]["LINEAR_API_KEY"], "sk-linear-test",
            "the provider's ${{VAR}} must resolve from the process env: {config}"
        );
        assert!(
            config["mcpServers"]["keel"]["env"].is_null(),
            "keel's own entry has no env — only a resolved provider does"
        );
    }

    #[test]
    fn claude_convergence_skips_disabled_mcp_provider() {
        // SAFETY: single-threaded test-only variable name.
        unsafe {
            std::env::set_var("KEEL_TEST_LINEAR_KEY", "sk-linear-test");
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let host_dir = root.join("host");
        std::fs::create_dir_all(&host_dir).unwrap();

        let manifest = AdapterManifest::for_client("claude").unwrap();
        let mut env = BTreeMap::new();
        let argv = wire_convergence(
            manifest.command.clone(),
            &mut env,
            &manifest,
            ConvergenceContext {
                host_dir: &host_dir,
                root,
                session_id: "session-test",
                skill_ids: &[],
                components: &disabled_linear_provider(),
            },
        )
        .unwrap();

        let mcp_path = argv
            .iter()
            .find(|a| a.ends_with("mcp.json"))
            .expect("a --mcp-config-style flag value pointing at the written file");
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(mcp_path).unwrap()).unwrap();
        assert!(
            config["mcpServers"]["keel"]["command"].is_string(),
            "keel's own entry must still be present: {config}"
        );
        assert!(
            config["mcpServers"]["linear"].is_null(),
            "disabled provider must not be injected into the client MCP config: {config}"
        );
    }

    #[test]
    fn codex_convergence_wires_a_configured_mcp_provider_alongside_keel() {
        unsafe {
            std::env::set_var("KEEL_TEST_LINEAR_KEY", "sk-linear-test");
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let host_dir = root.join("host");
        std::fs::create_dir_all(&host_dir).unwrap();

        let manifest = AdapterManifest::for_client("codex").unwrap();
        let mut env = BTreeMap::new();
        let argv = wire_convergence(
            manifest.command.clone(),
            &mut env,
            &manifest,
            ConvergenceContext {
                host_dir: &host_dir,
                root,
                session_id: "session-test",
                skill_ids: &["keel_verification".into()],
                components: &linear_provider(),
            },
        )
        .unwrap();

        assert!(
            argv.iter()
                .any(|a| a.starts_with("mcp_servers.keel.command=")),
            "keel's own entry must still be present: {argv:?}"
        );
        assert!(
            argv.iter()
                .any(|a| a == "mcp_servers.linear.command=\"sh\""),
            "the provider's command: {argv:?}"
        );
        assert!(
            argv.iter()
                .any(|a| a.contains("LINEAR_API_KEY") && a.contains("sk-linear-test")),
            "the provider's ${{VAR}} must resolve from the process env: {argv:?}"
        );
    }

    #[test]
    fn opencode_convergence_wires_a_configured_mcp_provider_alongside_keel() {
        unsafe {
            std::env::set_var("KEEL_TEST_LINEAR_KEY", "sk-linear-test");
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let host_dir = root.join("host");
        std::fs::create_dir_all(&host_dir).unwrap();

        let manifest = AdapterManifest::for_client("opencode").unwrap();
        let mut env = BTreeMap::new();
        wire_convergence(
            manifest.command.clone(),
            &mut env,
            &manifest,
            ConvergenceContext {
                host_dir: &host_dir,
                root,
                session_id: "session-test",
                skill_ids: &["keel_verification".into()],
                components: &linear_provider(),
            },
        )
        .unwrap();

        let config = env
            .get("OPENCODE_CONFIG_CONTENT")
            .expect("opencode config env is written");
        let config: serde_json::Value = serde_json::from_str(config).unwrap();
        assert!(
            config["mcp"]["keel"]["command"].is_array(),
            "keel's own entry must still be present: {config}"
        );
        assert_eq!(config["mcp"]["linear"]["command"][0], "sh");
        assert_eq!(config["mcp"]["linear"]["command"][1], "fake-linear-mcp.sh");
        assert_eq!(
            config["mcp"]["linear"]["environment"]["LINEAR_API_KEY"], "sk-linear-test",
            "the provider's ${{VAR}} must resolve from the process env: {config}"
        );
    }

    #[test]
    fn a_provider_missing_config_command_fails_convergence_loudly() {
        let mut components = BTreeMap::new();
        components.insert(
            "mcp-provider:broken".to_string(),
            CompiledComponent {
                kind: "mcp-provider".into(),
                id: "broken".into(),
                version: "0".into(),
                description: None,
                match_: CompiledMatch::default(),
                content: None,
                inline: None,
                requirements: vec![],
                capabilities: vec![],
                config: Some(serde_json::json!({"enabled": true})),
            },
        );
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let host_dir = root.join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let manifest = AdapterManifest::for_client("claude").unwrap();
        let mut env = BTreeMap::new();

        let err = wire_convergence(
            manifest.command.clone(),
            &mut env,
            &manifest,
            ConvergenceContext {
                host_dir: &host_dir,
                root,
                session_id: "session-test",
                skill_ids: &["keel_verification".into()],
                components: &components,
            },
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("broken"),
            "the error must name the misconfigured provider: {err:#}"
        );
    }
}
