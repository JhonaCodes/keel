// SPDX-License-Identifier: Apache-2.0
//! Resolves `kind: MCPProvider` components into MCP servers `wire_convergence`
//! injects into every launched client (H-011) — the multi-server sibling of
//! `executor_command`/`executor_env` for `kind: ModelExecutor`.

use crate::executor::resolve_env_map;
use keel_engine::snapshot::CompiledComponent;
use std::collections::BTreeMap;
use thiserror::Error;

/// One MCP server to wire into a launched client's config: keel's own entry,
/// plus one per authored `MCPProvider`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerSpec {
    pub name: String,
    pub command: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum McpProviderError {
    #[error(
        "MCP provider `{0}` has no `config.command` — a governed provider is a local stdio server (H-011)"
    )]
    MissingCommand(String),
    #[error("MCP provider id `keel` is reserved for keel's own entry")]
    ReservedName,
}

/// Resolves every explicitly enabled `kind: MCPProvider` component in the
/// compiled snapshot into an `McpServerSpec`, `${VAR}`-resolving `config.env`
/// the same way `executor_env` does for a `ModelExecutor` (`resolve_env_map`).
/// Providers are fail-closed: `config.enabled: true` is required. An omitted
/// or false value keeps an optional provider in the governed workspace without
/// injecting it into every launched client.
/// Errors loudly on an enabled provider missing `config.command` or reusing the
/// reserved `keel` id — an authored enabled provider that cannot be run is a
/// mistake to surface, not something to silently skip (unlike an unmapped hook
/// shape).
pub fn compiled_mcp_providers(
    components: &BTreeMap<String, CompiledComponent>,
) -> Result<Vec<McpServerSpec>, McpProviderError> {
    components
        .values()
        .filter(|c| c.kind == "mcp-provider")
        .map(|component| {
            if component
                .config
                .as_ref()
                .and_then(|c| c.get("enabled"))
                .and_then(|enabled| enabled.as_bool())
                != Some(true)
            {
                return Ok(None);
            }
            if component.id == "keel" {
                return Err(McpProviderError::ReservedName);
            }
            let command = component
                .config
                .as_ref()
                .and_then(|c| c.get("command"))
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                        .collect::<Vec<_>>()
                })
                .filter(|argv| !argv.is_empty())
                .ok_or_else(|| McpProviderError::MissingCommand(component.id.clone()))?;
            let env = component
                .config
                .as_ref()
                .and_then(|c| c.get("env"))
                .and_then(|e| e.as_object())
                .map(resolve_env_map)
                .unwrap_or_default();
            Ok(Some(McpServerSpec {
                name: component.id.clone(),
                command,
                env,
            }))
        })
        .filter_map(|result| result.transpose())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use keel_engine::snapshot::CompiledMatch;

    fn provider(id: &str, config: serde_json::Value) -> (String, CompiledComponent) {
        (
            format!("mcp-provider:{id}"),
            CompiledComponent {
                kind: "mcp-provider".into(),
                id: id.into(),
                version: "0".into(),
                description: None,
                match_: CompiledMatch::default(),
                content: None,
                inline: None,
                requirements: vec![],
                capabilities: vec![],
                config: Some(config),
            },
        )
    }

    #[test]
    fn resolves_command_and_dollar_var_env_for_each_provider() {
        // SAFETY: single-threaded test-only variable name.
        unsafe {
            std::env::set_var("KEEL_TEST_LINEAR_KEY", "sk-linear-test");
        }
        let mut components = BTreeMap::new();
        let (key, component) = provider(
            "linear",
            serde_json::json!({
                "enabled": true,
                "command": ["npx", "-y", "@linear/mcp-server"],
                "env": { "LINEAR_API_KEY": "${KEEL_TEST_LINEAR_KEY}" }
            }),
        );
        components.insert(key, component);

        let servers = compiled_mcp_providers(&components).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "linear");
        assert_eq!(
            servers[0].command,
            vec![
                "npx".to_string(),
                "-y".to_string(),
                "@linear/mcp-server".to_string()
            ]
        );
        assert_eq!(
            servers[0].env,
            vec![("LINEAR_API_KEY".to_string(), "sk-linear-test".to_string())]
        );
    }

    #[test]
    fn provider_without_explicit_enabled_is_not_wired() {
        let mut components = BTreeMap::new();
        let (key, component) = provider(
            "optional",
            serde_json::json!({"command": ["npx", "-y", "optional-mcp"]}),
        );
        components.insert(key, component);

        assert_eq!(compiled_mcp_providers(&components).unwrap(), Vec::new());
    }

    #[test]
    fn a_provider_with_no_command_errors_instead_of_disappearing() {
        let mut components = BTreeMap::new();
        let (key, component) = provider("broken", serde_json::json!({"enabled": true}));
        components.insert(key, component);

        assert_eq!(
            compiled_mcp_providers(&components),
            Err(McpProviderError::MissingCommand("broken".to_string()))
        );
    }

    #[test]
    fn disabled_provider_is_not_wired() {
        let mut components = BTreeMap::new();
        let (key, component) = provider(
            "workflow-local",
            serde_json::json!({
                "enabled": false,
                "command": ["npx", "-y", "mcp-remote", "http://127.0.0.1:8765/mcp"]
            }),
        );
        components.insert(key, component);

        assert_eq!(compiled_mcp_providers(&components).unwrap(), Vec::new());
    }

    #[test]
    fn a_provider_named_keel_is_rejected_as_reserved() {
        let mut components = BTreeMap::new();
        let (key, component) = provider(
            "keel",
            serde_json::json!({"enabled": true, "command": ["x"]}),
        );
        components.insert(key, component);

        assert_eq!(
            compiled_mcp_providers(&components),
            Err(McpProviderError::ReservedName)
        );
    }

    #[test]
    fn non_provider_components_are_ignored() {
        let mut components = BTreeMap::new();
        let (_, mut component) = provider("agent-cli", serde_json::json!({"command": ["x"]}));
        component.kind = "model-executor".into();
        components.insert("model-executor:agent-cli".to_string(), component);

        assert_eq!(compiled_mcp_providers(&components).unwrap(), Vec::new());
    }
}
