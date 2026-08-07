// SPDX-License-Identifier: Apache-2.0
//! Workspace loading — convention over configuration.
//!
//! Two shapes, one loader. A LAYER (or a flat workspace root) carries its
//! components by convention: `rules/`, `tools/`, `skills/`, `agents/`,
//! `tests/`, `exceptions/` — scanned by [`load_components`]. [`load_layered`]
//! reads a full section 8.5 workspace as its composition layers (`global/`,
//! `organizations/<org>/`, `platforms/`, `projects/<name>/`, `teams/`,
//! `profiles/`), in the fixed section 7.2 order; a flat workspace (components
//! under the root, no layer directories) loads as one degenerate `Project`
//! layer. `keel init` scaffolds the layered layout; [`load`] returns the flat
//! root as a single `WorkspaceFiles` (used by commands that read the published
//! snapshot).
//!
//! Execution state lives in an ignored directory INSIDE the workspace
//! (spec section 8.4 allows "outside the source tree or in an ignored directory")
//! for inspectability: `keel doctor` and the dev see the snapshot and ledger
//! next to the rules that produced them. XDG paths arrive with the installer.

use keel_dsl::{
    AgentDoc, Document, DslError, ExceptionDoc, GovernedComponentDoc, RuleDoc, RuleTestDoc,
    SkillDoc, ToolDoc, WorkspaceDoc, parse_documents,
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
    pub components: Vec<(String, GovernedComponentDoc)>,
    pub tests: Vec<RuleTestDoc>,
    /// Governed exceptions (section 7.4): the only route to relax a `locked`
    /// rule, owned at the locking scope with reason, bounded scope and expiry.
    pub exceptions: Vec<ExceptionDoc>,
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
            components: Vec::new(),
            tests: Vec::new(),
            exceptions: Vec::new(),
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
    #[error("no workspace.yaml in `{0}` — is this a Keel workspace? (keel init)")]
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
    #[error(
        "`{0}` mixes root-level components (rules/, tools/, ...) with composition layer directories (global/, organizations/, projects/, ...); a workspace is either flat or layered, not both — move the root components into a layer"
    )]
    MixedLayout(PathBuf),
    #[error(
        "`{path}` uses the org-native `components/` layout, which is not supported yet (deferred): author components under rules/, tools/, skills/, agents/ or tests/ instead"
    )]
    UnsupportedLayerLayout { path: PathBuf },
}

/// The AUTHORING component subdirectories whose presence at the ROOT marks a
/// flat (single-layer) workspace. `tests/` is deliberately EXCLUDED: the
/// section 8.5 layered layout carries a workspace-level `tests/` at the root
/// alongside the layer directories, so it must not be read as a flat marker.
const FLAT_MARKER_SUBDIRS: [&str; 12] = [
    "rules",
    "tools",
    "skills",
    "agents",
    "blueprints",
    "knowledge",
    "workflows",
    "contracts",
    "hooks",
    "providers",
    "policies",
    "executors",
];

/// Whether `dir` carries an authoring component subdirectory directly — the
/// marker of a flat/degenerate workspace root.
fn has_components(dir: &Path) -> bool {
    FLAT_MARKER_SUBDIRS.iter().any(|sub| dir.join(sub).is_dir())
}

/// Loads and validates (schema included) all workspace documents.
pub fn load(root: &Path) -> Result<WorkspaceFiles, WorkspaceError> {
    let ws_file = root.join("workspace.yaml");
    if !ws_file.exists() {
        return Err(WorkspaceError::NotAWorkspace(root.to_path_buf()));
    }

    let mut files = load_components(root)?;

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

    Ok(files)
}

/// Scans ONE directory's component subdirectories (`rules/`, `tools/`,
/// `skills/`, `agents/`, `tests/`) into a [`WorkspaceFiles`]. This is the unit
/// of a single composition LAYER (section 8.5): the same convention applies to
/// the flat root, to `global/` and to each `projects/<name>/` alike. It does
/// NOT require a `workspace.yaml` — that is a workspace-root concern, not a
/// per-layer one.
pub fn load_components(dir: &Path) -> Result<WorkspaceFiles, WorkspaceError> {
    let mut files = WorkspaceFiles::empty(dir.to_path_buf());

    for (sub, expect_hint) in [
        ("rules", "Rule"),
        ("tools", "Tool"),
        ("skills", "Skill"),
        ("agents", "Agent"),
        ("blueprints", "Blueprint"),
        ("knowledge", "Knowledge"),
        ("workflows", "Workflow"),
        ("contracts", "Contract"),
        ("hooks", "Hook"),
        ("providers", "MCPProvider"),
        ("policies", "Policy"),
        ("executors", "ModelExecutor"),
        ("tests", "RuleTest"),
        ("exceptions", "Exception"),
    ] {
        let dir_path = dir.join(sub);
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
                    (Document::AgentRoutingPolicy(x), "Agent") => files
                        .components
                        .push(("agent-routing-policy".to_string(), *x)),
                    (Document::Blueprint(x), "Blueprint") => {
                        files.components.push(("blueprint".to_string(), *x))
                    }
                    (Document::Knowledge(x), "Knowledge") => {
                        files.components.push(("knowledge".to_string(), *x))
                    }
                    (Document::Workflow(x), "Workflow") => {
                        files.components.push(("workflow".to_string(), *x))
                    }
                    (Document::Contract(x), "Contract") => {
                        files.components.push(("contract".to_string(), *x))
                    }
                    (Document::Hook(x), "Hook") => files.components.push(("hook".to_string(), *x)),
                    (Document::MCPProvider(x), "MCPProvider") => {
                        files.components.push(("mcp-provider".to_string(), *x))
                    }
                    (Document::Policy(x), "Policy") => {
                        files.components.push(("policy".to_string(), *x))
                    }
                    (Document::ModelExecutor(x), "ModelExecutor") => {
                        files.components.push(("model-executor".to_string(), *x))
                    }
                    (Document::RuleTest(t), "RuleTest") => files.tests.push(*t),
                    (Document::Exception(e), "Exception") => files.exceptions.push(*e),
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
        .tests
        .sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
    files
        .exceptions
        .sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
    files
        .components
        .sort_by(|a, b| (&a.0, &a.1.metadata.id).cmp(&(&b.0, &b.1.metadata.id)));

    Ok(files)
}

/// The authority layers of the composition chain (spec section 7.2), highest
/// authority FIRST. The derived `Ord` IS the composition order: composition
/// runs global → organization → platform → project → team → profile, and a
/// lower-authority layer may only ADD restriction (section 7.4), never weaken a
/// higher one. The `task/session` layer of section 7.2 is runtime-only
/// (append-only, non-authoritative, section 7.5) and has no directory.
///
/// Do NOT reorder these variants: the ordering is load-bearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayerId {
    Global,
    Organization,
    Platform,
    Project,
    Team,
    Profile,
}

impl LayerId {
    /// The composition chain, highest authority first.
    pub const CHAIN: [LayerId; 6] = [
        LayerId::Global,
        LayerId::Organization,
        LayerId::Platform,
        LayerId::Project,
        LayerId::Team,
        LayerId::Profile,
    ];

    /// The workspace subdirectory that holds this layer (section 8.5). `global/`
    /// carries its components directly; the rest namespace instances under a
    /// plural directory (`organizations/<org>/`, `projects/<proj>/`, ...).
    pub fn dir(self) -> &'static str {
        match self {
            LayerId::Global => "global",
            LayerId::Organization => "organizations",
            LayerId::Platform => "platforms",
            LayerId::Project => "projects",
            LayerId::Team => "teams",
            LayerId::Profile => "profiles",
        }
    }

    /// Whether this layer namespaces instances by a subdirectory name.
    pub fn is_namespaced(self) -> bool {
        !matches!(self, LayerId::Global)
    }
}

/// One loaded composition layer: its authority position, its instance name
/// (the `organizations/<name>` / `projects/<name>` subdir; `None` for `global/`),
/// and the components it contributes.
#[derive(Debug)]
pub struct Layer {
    pub id: LayerId,
    pub name: Option<String>,
    pub files: WorkspaceFiles,
}

/// A workspace loaded as its composition layers (section 8.5), in composition
/// order (section 7.2). This is what a compiler composes into a single effective
/// rule set; which layers actually apply to a given repository is resolution
/// (section 7.1, a later step) — this loader simply reads every layer present.
#[derive(Debug)]
pub struct LayeredWorkspace {
    pub root: PathBuf,
    pub layers: Vec<Layer>,
}

/// Loads a workspace as its composition layers (spec section 8.5). Reads every
/// layer directory present (`global/`, `organizations/<org>/`,
/// `platforms/<p>/`, `projects/<proj>/`, `teams/<t>/`, `profiles/<name>/`) via
/// [`load_components`] — that is, the `rules/tools/skills/agents/tests`
/// convention — and returns them in composition order.
///
/// A FLAT workspace (components under the root, no layer directories) is read
/// as a single degenerate `Project` layer, so single-project workspaces keep
/// working unchanged. A workspace that mixes both shapes is rejected
/// ([`WorkspaceError::MixedLayout`]) rather than silently dropping the root
/// components.
///
/// SCOPE: only the component-directory convention is loaded. The org-native
/// section 8.5 layout (`components/{policies,contracts,workflows,permissions}/`)
/// and the org files `composition.yaml`/`repositories.yaml` are NOT read as
/// components here; a layer dir carrying a `components/` subdir is rejected
/// ([`WorkspaceError::UnsupportedLayerLayout`]) so nothing is dropped silently.
/// (`repositories.yaml` is read where it is used — resolution, section 7.1.)
pub fn load_layered(root: &Path) -> Result<LayeredWorkspace, WorkspaceError> {
    let ws_file = root.join("workspace.yaml");
    if !ws_file.exists() {
        return Err(WorkspaceError::NotAWorkspace(root.to_path_buf()));
    }
    for doc in parse_file(&ws_file)? {
        if !matches!(doc, Document::Workspace(_)) {
            return Err(WorkspaceError::MisplacedKind {
                path: ws_file.clone(),
                kind: doc.kind_name().to_string(),
            });
        }
    }

    let mut layers = Vec::new();
    for id in LayerId::CHAIN {
        let base = root.join(id.dir());
        if !base.is_dir() {
            continue;
        }
        if id.is_namespaced() {
            for name in sorted_subdirs(&base)? {
                let dir = base.join(&name);
                reject_unsupported_layout(&dir)?;
                layers.push(Layer {
                    id,
                    name: Some(name),
                    files: load_components(&dir)?,
                });
            }
        } else {
            reject_unsupported_layout(&base)?;
            layers.push(Layer {
                id,
                name: None,
                files: load_components(&base)?,
            });
        }
    }

    // A flat single-project workspace (components under the root, no layer
    // directories) composes as one degenerate Project layer.
    let root_is_flat = has_components(root);
    if !layers.is_empty() && root_is_flat {
        // Both a layered tree AND root components: ambiguous — never drop one.
        return Err(WorkspaceError::MixedLayout(root.to_path_buf()));
    }
    if layers.is_empty() && root_is_flat {
        layers.push(Layer {
            id: LayerId::Project,
            name: None,
            files: load_components(root)?,
        });
    }

    layers.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.name.cmp(&b.name)));
    Ok(LayeredWorkspace {
        root: root.to_path_buf(),
        layers,
    })
}

/// Rejects a layer directory whose org-native `components/` holds REAL
/// (loadable) content, so it is never silently ignored. A `components/` that is
/// empty or holds only docs/templates (`README.md`, `*.example`) is tolerated —
/// it is the documented scaffold of a future org-scale layer, with nothing to
/// drop. Real authored YAML there is rejected loudly (the layout is not
/// supported yet).
fn reject_unsupported_layout(layer_dir: &Path) -> Result<(), WorkspaceError> {
    let components = layer_dir.join("components");
    if components.is_dir() && dir_has_loadable_yaml(&components)? {
        return Err(WorkspaceError::UnsupportedLayerLayout {
            path: layer_dir.to_path_buf(),
        });
    }
    Ok(())
}

/// Whether `dir` (recursively) contains any authored YAML — a `.yaml`/`.yml`
/// that is not a `.example` template. Hidden entries are skipped.
fn dir_has_loadable_yaml(dir: &Path) -> Result<bool, WorkspaceError> {
    let entries = std::fs::read_dir(dir).map_err(|source| WorkspaceError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_str().unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if dir_has_loadable_yaml(&path)? {
                return Ok(true);
            }
        } else if (name.ends_with(".yaml") || name.ends_with(".yml")) && !name.ends_with(".example")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Immediate subdirectory names of `dir`, sorted (deterministic layer order,
/// invariant 9). Hidden entries (dot-prefixed, e.g. `.keel-state`) are skipped.
fn sorted_subdirs(dir: &Path) -> Result<Vec<String>, WorkspaceError> {
    let mut names = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|source| WorkspaceError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str()
            && !name.starts_with('.')
        {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
#[path = "../tests-unit/workspace.rs"]
mod tests;

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
