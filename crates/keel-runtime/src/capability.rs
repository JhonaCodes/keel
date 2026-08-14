use keel_core::Decision;
use keel_core::event::{Event, EventKind};
use keel_engine::runtime::{Mode, evaluate_event};
use keel_engine::snapshot::Snapshot;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityRequest {
    pub name: String,
    pub arguments: serde_json::Value,
}

impl CapabilityRequest {
    pub fn new(name: impl Into<String>, arguments: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            arguments,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityResult {
    pub name: String,
    pub output: serde_json::Value,
}

#[derive(Debug, Error)]
pub enum CapabilityError {
    #[error("capability `{name}` is not granted in this session/phase")]
    NotGranted { name: String },
    #[error("capability `{name}` is not implemented")]
    Unsupported { name: String },
    #[error("capability `{name}` denied before execution by rules {rules:?}")]
    PolicyDenied { name: String, rules: Vec<String> },
    #[error("path `{path}` escapes the governed workspace")]
    OutsideWorkspace { path: String },
    #[error("invalid capability arguments: {0}")]
    InvalidArguments(String),
    #[error("capability I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub struct CapabilityManager {
    root: PathBuf,
    grants: BTreeSet<String>,
    snapshot: Option<Snapshot>,
}

impl CapabilityManager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            grants: BTreeSet::new(),
            snapshot: None,
        }
    }

    pub fn from_snapshot(root: impl Into<PathBuf>, snapshot: &Snapshot) -> Self {
        Self {
            root: root.into(),
            grants: snapshot
                .components
                .values()
                .flat_map(|component| component.capabilities.iter().cloned())
                .collect(),
            snapshot: Some(snapshot.clone()),
        }
    }

    pub fn grant(&mut self, name: impl Into<String>) {
        self.grants.insert(name.into());
    }

    pub fn grants(&self) -> impl Iterator<Item = &str> {
        self.grants.iter().map(String::as_str)
    }

    pub fn execute(
        &self,
        request: &CapabilityRequest,
    ) -> Result<CapabilityResult, CapabilityError> {
        if !self.grants.contains(&request.name) {
            return Err(CapabilityError::NotGranted {
                name: request.name.clone(),
            });
        }
        self.enforce_policy(request)?;
        match request.name.as_str() {
            "filesystem.read" => self.read_file(request),
            "filesystem.write" => self.write_file(request),
            "shell.run" => self.run_process(request, None),
            "git.run" => self.run_process(request, Some("git")),
            other => Err(CapabilityError::Unsupported {
                name: other.to_string(),
            }),
        }
    }

    fn enforce_policy(&self, request: &CapabilityRequest) -> Result<(), CapabilityError> {
        let Some(snapshot) = &self.snapshot else {
            return Ok(());
        };
        let event = match request.name.as_str() {
            "filesystem.write" => Event {
                kind: EventKind::FileEdited,
                session_id: None,
                file: request.arguments["path"].as_str().map(ToOwned::to_owned),
                language: None,
                content: request.arguments["content"].as_str().map(ToOwned::to_owned),
                line: None,
                command: None,
                env: Default::default(),
                files: Vec::new(),
                loaded_skills: Vec::new(),
                recorded_evidence: Vec::new(),
                audit_scope: None,
                audit_mode: None,
                recorded_audits: Vec::new(),
            },
            "shell.run" | "git.run" => Event {
                kind: EventKind::CommandRequested,
                session_id: None,
                file: None,
                language: None,
                content: None,
                line: None,
                command: request.arguments["command"].as_array().map(|parts| {
                    let mut command = parts
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>();
                    if request.name == "git.run" {
                        command.insert(0, "git");
                    }
                    command.join(" ")
                }),
                env: Default::default(),
                files: Vec::new(),
                loaded_skills: Vec::new(),
                recorded_evidence: Vec::new(),
                audit_scope: None,
                audit_mode: None,
                recorded_audits: Vec::new(),
            },
            _ => return Ok(()),
        };
        let denied = evaluate_event(snapshot, &event, &self.root, Mode::Enforce)
            .into_iter()
            .filter(|evaluation| evaluation.effective_decision >= Decision::DenyPendingApproval)
            .map(|evaluation| evaluation.rule_id)
            .collect::<Vec<_>>();
        if denied.is_empty() {
            Ok(())
        } else {
            Err(CapabilityError::PolicyDenied {
                name: request.name.clone(),
                rules: denied,
            })
        }
    }

    fn read_file(&self, request: &CapabilityRequest) -> Result<CapabilityResult, CapabilityError> {
        let path = self.resolve_path(argument_str(&request.arguments, "path")?)?;
        let content = std::fs::read_to_string(path)?;
        Ok(CapabilityResult {
            name: request.name.clone(),
            output: serde_json::json!({ "content": content }),
        })
    }

    fn write_file(&self, request: &CapabilityRequest) -> Result<CapabilityResult, CapabilityError> {
        let relative = argument_str(&request.arguments, "path")?;
        let path = self.resolve_path(relative)?;
        let content = argument_str(&request.arguments, "content")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        Ok(CapabilityResult {
            name: request.name.clone(),
            output: serde_json::json!({ "path": relative, "bytes": content.len() }),
        })
    }

    fn run_process(
        &self,
        request: &CapabilityRequest,
        fixed_program: Option<&str>,
    ) -> Result<CapabilityResult, CapabilityError> {
        let values = request
            .arguments
            .get("command")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                CapabilityError::InvalidArguments("`command` must be an array".into())
            })?;
        let command = values
            .iter()
            .map(|value| {
                value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                    CapabilityError::InvalidArguments("command entries must be strings".into())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (program, args) = match fixed_program {
            Some(program) => (program, command.as_slice()),
            None => command
                .split_first()
                .map(|(program, args)| (program.as_str(), args))
                .ok_or_else(|| CapabilityError::InvalidArguments("command is empty".into()))?,
        };
        let mut process = Command::new(program);
        process.current_dir(&self.root).env_clear().args(args);
        if let Some(path) = std::env::var_os("PATH") {
            process.env("PATH", path);
        }
        let output = process.output()?;
        Ok(CapabilityResult {
            name: request.name.clone(),
            output: serde_json::json!({
                "status": output.status.code(),
                "success": output.status.success(),
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
            }),
        })
    }

    fn resolve_path(&self, relative: &str) -> Result<PathBuf, CapabilityError> {
        let path = Path::new(relative);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(CapabilityError::OutsideWorkspace {
                path: relative.to_string(),
            });
        }
        let root = self.root.canonicalize()?;
        let candidate = self.root.join(path);
        let mut existing = candidate.as_path();
        while std::fs::symlink_metadata(existing).is_err() {
            existing = existing
                .parent()
                .ok_or_else(|| CapabilityError::OutsideWorkspace {
                    path: relative.to_string(),
                })?;
        }
        if !existing.canonicalize()?.starts_with(&root) {
            return Err(CapabilityError::OutsideWorkspace {
                path: relative.to_string(),
            });
        }
        Ok(candidate)
    }
}

fn argument_str<'a>(
    arguments: &'a serde_json::Value,
    name: &str,
) -> Result<&'a str, CapabilityError> {
    arguments
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CapabilityError::InvalidArguments(format!("`{name}` must be a string")))
}
