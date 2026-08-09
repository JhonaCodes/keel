// SPDX-License-Identifier: Apache-2.0
//! Resolution by repository identity (spec section 7.1) — selecting WHICH
//! composition layers apply to a bound repository, in composition order
//! (section 7.2), before the compiler composes them (section 7.4).
//!
//! A repository carries only its binding (`.keel/project.yaml`: `project` +
//! `workspace`, invariant 4). Resolution reads that binding against a loaded
//! [`LayeredWorkspace`] and returns the ordered subset of layers that make up
//! the project's effective configuration:
//!
//! - `global/` always applies (the user/base layer);
//! - the `organizations/<org>/` named by the binding;
//! - every `packages/<tech>/` (invariant 3: reusable technology bundles) — they
//!   compose for every project, and each component's own `scope`/`match` decides
//!   where it actually applies (a Flutter package's dart-scoped rules never
//!   touch a Rust repo), so no per-project selector is needed yet;
//! - the `projects/<name>/` the binding points at (a flat single-project
//!   workspace's degenerate Project layer matches too).
//!
//! `platforms/`, `teams/` and `profiles/` selection has no documented
//! standalone selector (they are org-scale refinements that may only ADD
//! restriction, never weaken a `locked` rule); they are NOT silently included.
//! Their selection arrives with the org-scale install story.
//!
//! Identity is also VERIFIED here (section 7.1 / section 13.3): if the resolved
//! organization ships a `repositories.yaml`, the repo's own git identity is
//! checked against the declared project. Locally this is ADVISORY — a mismatch
//! degrades to a warning, never a hard block (section 13.3, compliance-plane
//! concern); it becomes enforcing in CI.

use crate::lock::ProjectBinding;
use crate::workspace::{Layer, LayerId, LayeredWorkspace};
use keel_dsl::{Document, DslError, RepositoryRegistrySpec, parse_documents};

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("malformed project reference `{0}` — expected `project:<org>/<name>`")]
    BadProjectRef(String),
    #[error("I/O error reading `{path}`: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("in `{path}`: {source}")]
    Dsl {
        path: std::path::PathBuf,
        #[source]
        source: DslError,
    },
}

/// Whether the repo's git identity matches its declared project (section 7.1).
/// Local resolution is cooperative, so a disagreement is advisory (section 13.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityStatus {
    /// No registry to check against (a local/standalone workspace).
    Unregistered,
    /// The registry maps the repo's identity to exactly the bound project.
    Verified,
    /// The registry disagrees with the binding, or the repo is absent from it.
    /// Locally advisory (section 13.3); CI turns this into a failure.
    Advisory(String),
}

/// The ordered layers that apply to a bound project, plus the identity check.
#[derive(Debug)]
pub struct ResolvedChain {
    /// Indices into [`LayeredWorkspace::layers`], in composition order (section 7.2).
    pub layer_indices: Vec<usize>,
    pub identity: IdentityStatus,
    /// Whether a Project layer actually matched the binding. `false` means the
    /// bound project contributes NOTHING — the caller must surface that (a repo
    /// bound to a project that enforces nothing is a silent gap, section 7.1),
    /// never treat the thin chain as a normal result.
    pub matched_project: bool,
}

impl ResolvedChain {
    /// The resolved layers, highest authority first (composition order).
    pub fn layers<'a>(&self, ws: &'a LayeredWorkspace) -> impl Iterator<Item = &'a Layer> {
        self.layer_indices.iter().map(|&i| &ws.layers[i])
    }
}

/// Resolves the composition chain for `binding` over `layered`.
///
/// `repo_identity` is the repository's own git identity (`<org>/<name>`, e.g.
/// `NuiMarkets/con-app`), used only for the section 7.1 verification; pass `None`
/// when it is unknown (the check then reports `Unregistered`).
pub fn resolve(
    layered: &LayeredWorkspace,
    binding: &ProjectBinding,
    repo_identity: Option<&str>,
) -> Result<ResolvedChain, ResolveError> {
    let (project_org, project_name) = parse_project(&binding.project)
        .ok_or_else(|| ResolveError::BadProjectRef(binding.project.clone()))?;
    // `workspace:` (e.g. `org:nui`) names the owning organization; fall back to
    // the org embedded in the project reference.
    let org = workspace_org(&binding.workspace).unwrap_or_else(|| project_org.clone());

    // layered.layers is already sorted in composition order, so preserving its
    // order while filtering yields the chain highest-authority first.
    let mut layer_indices = Vec::new();
    for (i, layer) in layered.layers.iter().enumerate() {
        let selected = match layer.id {
            LayerId::Global => true,
            LayerId::Organization => layer.name.as_deref() == Some(org.as_str()),
            LayerId::Project => project_layer_matches(layer, &project_name),
            // Technology packages (invariant 3) compose for every project; their
            // per-component `scope`/`match` decide applicability (a Flutter
            // package's dart-scoped rules never touch a Rust repo), so no
            // per-project selector is needed. Versioned pinning stays deferred.
            LayerId::Package => true,
            // No documented standalone selector — deferred, not silently taken.
            LayerId::Platform | LayerId::Team | LayerId::Profile => false,
        };
        if selected {
            layer_indices.push(i);
        }
    }

    let matched_project = layer_indices
        .iter()
        .any(|&i| layered.layers[i].id == LayerId::Project);
    let identity = check_identity(layered, &org, binding, repo_identity)?;
    Ok(ResolvedChain {
        layer_indices,
        identity,
        matched_project,
    })
}

/// A Project layer applies if its instance name matches, OR if it is the
/// unnamed degenerate layer of a flat single-project workspace.
fn project_layer_matches(layer: &Layer, project_name: &str) -> bool {
    match &layer.name {
        Some(name) => name == project_name,
        None => true,
    }
}

/// `project:<org>/<name>` → `(org, name)`.
fn parse_project(reference: &str) -> Option<(String, String)> {
    let rest = reference.strip_prefix("project:")?;
    let (org, name) = rest.split_once('/')?;
    if org.is_empty() || name.is_empty() {
        return None;
    }
    Some((org.to_string(), name.to_string()))
}

/// `org:<name>` → `name`. Other workspace-reference shapes yield `None` (the
/// caller falls back to the project's own org).
fn workspace_org(workspace: &str) -> Option<String> {
    workspace
        .strip_prefix("org:")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn check_identity(
    layered: &LayeredWorkspace,
    org: &str,
    binding: &ProjectBinding,
    repo_identity: Option<&str>,
) -> Result<IdentityStatus, ResolveError> {
    let Some(registry) = load_repository_registry(layered, org)? else {
        return Ok(IdentityStatus::Unregistered);
    };
    // A registry EXISTS but we cannot determine the repo's identity: that is a
    // "cannot verify" condition (advisory), distinct from a standalone
    // workspace with no registry at all (Unregistered).
    let Some(repo_identity) = repo_identity else {
        return Ok(IdentityStatus::Advisory(format!(
            "`{org}` ships a repositories.yaml but the repository identity could not be determined to verify it"
        )));
    };
    match registry.repositories.iter().find(|e| e.id == repo_identity) {
        None => Ok(IdentityStatus::Advisory(format!(
            "repository `{repo_identity}` is not registered in `{org}` repositories.yaml"
        ))),
        Some(entry) if entry.project == binding.project => Ok(IdentityStatus::Verified),
        Some(entry) => Ok(IdentityStatus::Advisory(format!(
            "repository `{repo_identity}` maps to `{}`, but the binding declares `{}`",
            entry.project, binding.project
        ))),
    }
}

/// Reads `organizations/<org>/repositories.yaml` if present (spec section 8.5).
fn load_repository_registry(
    layered: &LayeredWorkspace,
    org: &str,
) -> Result<Option<RepositoryRegistrySpec>, ResolveError> {
    let path = layered
        .root
        .join("organizations")
        .join(org)
        .join("repositories.yaml");
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path).map_err(|source| ResolveError::Io {
        path: path.clone(),
        source,
    })?;
    let docs = parse_documents(&raw).map_err(|source| ResolveError::Dsl {
        path: path.clone(),
        source,
    })?;
    for doc in docs {
        if let Document::RepositoryRegistry(reg) = doc {
            return Ok(Some(reg.spec));
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "../tests-unit/resolution.rs"]
mod tests;
