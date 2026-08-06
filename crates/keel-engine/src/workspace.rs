// SPDX-License-Identifier: Apache-2.0
//! Workspace loading — convention over configuration (Phase 0).
//!
//! Minimal structure (spec section 8.5 trimmed down to a single project without
//! layers):
//!
//! ```text
//! <workspace>/
//! ├── workspace.yaml      # kind: Workspace (required)
//! ├── rules/*.yaml        # kind: Rule
//! ├── tools/*.yaml        # kind: Tool (manifests for external tools)
//! ├── skills/*.yaml       # kind: Skill (compact/full/examples, section 14.12)
//! ├── agents/*.yaml       # kind: Agent + AgentExecutor (section 14)
//! ├── tests/*.yaml        # kind: RuleTest (compile gate, section 10.2)
//! ├── fixtures/*.jsonl    # replay events for `keel observe`
//! └── .keel-state/        # execution state: snapshot, ledger (IGNORED)
//! ```
//!
//! Execution state lives in an ignored directory INSIDE the workspace
//! (spec section 8.4 allows "outside the source tree or in an ignored directory").
//! Workspace-local is chosen for Phase 0 for inspectability: `keel doctor`
//! and the dev see the snapshot and ledger right next to the rules that
//! produced them. XDG paths arrive with the installation story (Phase 1).

use keel_dsl::{
    AgentDoc, AgentExecutorDoc, Document, DslError, RuleDoc, RuleTestDoc, SkillDoc, ToolDoc,
    WorkspaceDoc, parse_documents,
};
use std::path::{Path, PathBuf};

pub const STATE_DIR: &str = ".keel-state";
pub const SNAPSHOT_FILE: &str = "snapshot.json";
pub const SNAPSHOT_PREV_FILE: &str = "snapshot.prev.json";
pub const LEDGER_FILE: &str = "ledger.sqlite";

#[derive(Debug)]
pub struct WorkspaceFiles {
    pub root: PathBuf,
    pub workspace: Option<WorkspaceDoc>,
    pub rules: Vec<RuleDoc>,
    pub tools: Vec<ToolDoc>,
    pub skills: Vec<SkillDoc>,
    pub agents: Vec<AgentDoc>,
    pub executors: Vec<AgentExecutorDoc>,
    pub tests: Vec<RuleTestDoc>,
}

impl WorkspaceFiles {
    pub fn empty(root: PathBuf) -> Self {
        WorkspaceFiles {
            root,
            workspace: None,
            rules: Vec::new(),
            tools: Vec::new(),
            skills: Vec::new(),
            agents: Vec::new(),
            executors: Vec::new(),
            tests: Vec::new(),
        }
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root.join(STATE_DIR)
    }
    pub fn snapshot_path(&self) -> PathBuf {
        self.state_dir().join(SNAPSHOT_FILE)
    }
    pub fn snapshot_prev_path(&self) -> PathBuf {
        self.state_dir().join(SNAPSHOT_PREV_FILE)
    }
    pub fn ledger_path(&self) -> PathBuf {
        self.state_dir().join(LEDGER_FILE)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("no workspace.yaml in `{0}` — is this an Keel workspace? (keel workspace init)")]
    NotAWorkspace(PathBuf),
    #[error("I/O error reading `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("in `{path}`: {source}")]
    Dsl {
        path: PathBuf,
        #[source]
        source: DslError,
    },
    #[error(
        "`{path}` contains a `{kind}` — kinds belong in their directory: rules/, tools/, skills/, tests/"
    )]
    MisplacedKind { path: PathBuf, kind: String },
}

/// Loads and validates (schema included) all workspace documents.
pub fn load(root: &Path) -> Result<WorkspaceFiles, WorkspaceError> {
    let ws_file = root.join("workspace.yaml");
    if !ws_file.exists() {
        return Err(WorkspaceError::NotAWorkspace(root.to_path_buf()));
    }

    let mut files = WorkspaceFiles::empty(root.to_path_buf());

    for doc in parse_file(&ws_file)? {
        match doc {
            Document::Workspace(w) => files.workspace = Some(*w),
            other => {
                return Err(WorkspaceError::MisplacedKind {
                    path: ws_file.clone(),
                    kind: other.kind_name().to_string(),
                });
            }
        }
    }

    for (dir, expect_hint) in [
        ("rules", "Rule"),
        ("tools", "Tool"),
        ("skills", "Skill"),
        ("agents", "Agent"),
        ("tests", "RuleTest"),
    ] {
        let dir_path = root.join(dir);
        if !dir_path.is_dir() {
            continue;
        }
        for path in yaml_files(&dir_path)? {
            for doc in parse_file(&path)? {
                match (doc, expect_hint) {
                    (Document::Rule(r), "Rule") => files.rules.push(*r),
                    (Document::Tool(t), "Tool") => files.tools.push(*t),
                    (Document::Skill(k), "Skill") => files.skills.push(*k),
                    (Document::Agent(a), "Agent") => files.agents.push(*a),
                    // Executors live alongside agents in agents/.
                    (Document::AgentExecutor(x), "Agent") => files.executors.push(*x),
                    (Document::RuleTest(t), "RuleTest") => files.tests.push(*t),
                    (other, _) => {
                        return Err(WorkspaceError::MisplacedKind {
                            path: path.clone(),
                            kind: other.kind_name().to_string(),
                        });
                    }
                }
            }
        }
    }

    // Deterministic ordering by id: the snapshot hash must not depend on the
    // filesystem's listing order (invariant 9).
    files
        .rules
        .sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
    files
        .tools
        .sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
    files
        .skills
        .sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
    files
        .agents
        .sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
    files
        .executors
        .sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
    files
        .tests
        .sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));

    Ok(files)
}

fn yaml_files(dir: &Path) -> Result<Vec<PathBuf>, WorkspaceError> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|source| WorkspaceError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if matches!(ext, "yaml" | "yml") {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn parse_file(path: &Path) -> Result<Vec<Document>, WorkspaceError> {
    let raw = std::fs::read_to_string(path).map_err(|source| WorkspaceError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_documents(&raw).map_err(|source| WorkspaceError::Dsl {
        path: path.to_path_buf(),
        source,
    })
}
