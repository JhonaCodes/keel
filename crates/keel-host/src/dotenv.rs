// SPDX-License-Identifier: Apache-2.0
//! Load a workspace `.env` into the process environment for `${VAR}` secrets.
//!
//! Keel resolves `${VAR}` in `ModelExecutor.config.env` (and, once dispatched,
//! MCP provider configs) from the PROCESS environment. This lets an operator
//! keep secrets in a gitignored `<workspace>/.env` instead of exporting them in
//! the shell: the governed processes (`keel launch`/`mcp`/`gate`) load it before
//! any `${VAR}` is resolved. A variable already present in the real environment
//! is NEVER overwritten — a shell export always wins over the file.
//!
//! This only makes the secrets available to keel's own `${VAR}` resolution; a
//! `CliModelExecutor` still runs with `env_clear` + PATH + ONLY the vars its
//! `config.env` declares, so loading the file does not hand the whole secret set
//! to an agent.

use std::path::Path;

/// Loads `<root>/.env` into the process environment (best-effort). Missing file
/// → no-op. Malformed lines are skipped with a diagnostic, never fatal.
pub fn load_workspace_env(root: &Path) {
    let path = root.join(".env");
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return, // no .env is the common case, not an error
    };
    for (name, value) in parse(&contents) {
        // Precedence: a real shell export wins; the file never clobbers it.
        if std::env::var_os(&name).is_none() {
            // SAFETY: set at startup, before threads that read the environment
            // are spawned (the governed session's children inherit it).
            unsafe {
                std::env::set_var(&name, &value);
            }
        }
    }
}

/// Parses `KEY=VALUE` lines. Ignores blanks and `#` comments; tolerates an
/// `export ` prefix; strips one layer of surrounding single/double quotes from
/// the value. A line without `=` or with an empty key is skipped.
fn parse(contents: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = strip_quotes(value.trim());
        out.push((key.to_string(), value.to_string()));
    }
    out
}

/// Removes one matching pair of surrounding single or double quotes.
fn strip_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_handles_comments_export_prefix_and_quotes() {
        let env = "\
# a comment
export API_KEY=\"sk-abc123\"
PLAIN=value
  SPACED = trimmed
QUOTED='single'
noequals
=novalue
";
        let parsed = parse(env);
        assert_eq!(
            parsed,
            vec![
                ("API_KEY".to_string(), "sk-abc123".to_string()),
                ("PLAIN".to_string(), "value".to_string()),
                ("SPACED".to_string(), "trimmed".to_string()),
                ("QUOTED".to_string(), "single".to_string()),
            ],
            "comments/blank/no-`=`/empty-key are skipped; export + quotes handled"
        );
    }

    #[test]
    fn load_sets_missing_vars_but_never_overwrites_a_real_export() {
        let dir = std::env::temp_dir().join(format!("keel-dotenv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".env"),
            "KEEL_DOTENV_NEW=fromfile\nKEEL_DOTENV_EXISTING=fromfile\n",
        )
        .unwrap();

        // Simulate a shell export that must win over the file.
        // SAFETY: single-threaded unit test with test-only variable names.
        unsafe {
            std::env::set_var("KEEL_DOTENV_EXISTING", "fromshell");
        }

        load_workspace_env(&dir);

        assert_eq!(std::env::var("KEEL_DOTENV_NEW").unwrap(), "fromfile");
        assert_eq!(
            std::env::var("KEEL_DOTENV_EXISTING").unwrap(),
            "fromshell",
            "a variable already in the environment must not be clobbered by .env"
        );

        // A missing .env directory is a silent no-op.
        load_workspace_env(&dir.join("does-not-exist"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The user's exact requirement, end to end: a secret in `<workspace>/.env`
    /// is resolved by a ModelExecutor's `config.env: { X: "${X}" }` — without
    /// being exported in the shell. Proves the `.env` → load → `executor_env`
    /// chain across crates.
    #[test]
    fn a_secret_in_dotenv_resolves_through_executor_config_env() {
        use keel_engine::snapshot::CompiledComponent;
        use keel_runtime::executor_env;
        use std::collections::BTreeMap;

        let dir = std::env::temp_dir().join(format!("keel-dotenv-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".env"), "KEEL_E2E_API_KEY=sk-from-dotenv\n").unwrap();

        // Not exported in the shell — only in the .env.
        assert!(std::env::var("KEEL_E2E_API_KEY").is_err());
        load_workspace_env(&dir);

        let mut components = BTreeMap::new();
        components.insert(
            "model-executor:agent-cli".to_string(),
            CompiledComponent {
                kind: "ModelExecutor".into(),
                id: "agent-cli".into(),
                version: "0".into(),
                content: None,
                inline: None,
                requirements: vec![],
                capabilities: vec![],
                config: Some(serde_json::json!({
                    "command": ["some-cli"],
                    "env": { "API_KEY": "${KEEL_E2E_API_KEY}" }
                })),
            },
        );

        let env = executor_env(&components, "agent-cli");
        assert_eq!(
            env,
            vec![("API_KEY".to_string(), "sk-from-dotenv".to_string())],
            "the executor resolves its ${{VAR}} from the loaded .env secret"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
