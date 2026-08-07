// SPDX-License-Identifier: Apache-2.0
//! Repository binding + resolution lock (spec section 8.6, invariants 4 & 9).
//!
//! The code repository holds ONLY the binding (`.keel/project.yaml`) and the
//! pinned resolution (`.keel/keel.lock`) — never the component definitions
//! (invariant 4). The lock's core is the snapshot's canonical hash: the SAME
//! configuration yields the SAME hash on any machine, so a CI plane that
//! recompiles and compares turns `locked` into a real guarantee (invariant 9).
//!
//! Determinism: the lock carries no timestamps and its component lists are
//! sorted, so local and CI produce byte-identical locks from the same inputs.

use crate::snapshot::Snapshot;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Directory (versioned, unlike `.keel-state/`) that holds the binding + lock.
pub const BINDING_DIR: &str = ".keel";
pub const BINDING_FILE: &str = "project.yaml";
pub const LOCK_FILE: &str = "keel.lock";

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("no binding at `{0}` — run `keel bind` first")]
    NoBinding(PathBuf),
    #[error("no lock at `{0}` — run `keel lock` first")]
    NoLock(PathBuf),
    #[error("I/O error on `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("malformed YAML in `{path}`: {source}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml_ng::Error,
    },
}

/// `.keel/project.yaml` — the only binding versioned in the repo (section 8.6):
/// which project this repo is, and which workspace resolves its components.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectBinding {
    /// Project identity, e.g. `project:org/repo` (repo-identity resolution, section 7.1).
    pub project: String,
    /// Workspace reference that owns the definitions, e.g. `org:local`.
    pub workspace: String,
}

impl ProjectBinding {
    pub fn path(root: &Path) -> PathBuf {
        root.join(BINDING_DIR).join(BINDING_FILE)
    }

    pub fn load(root: &Path) -> Result<Self, LockError> {
        let path = Self::path(root);
        if !path.exists() {
            return Err(LockError::NoBinding(path));
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| LockError::Io {
            path: path.clone(),
            source: e,
        })?;
        serde_yaml_ng::from_str(&raw).map_err(|e| LockError::Yaml { path, source: e })
    }

    pub fn write(&self, root: &Path) -> Result<(), LockError> {
        let dir = root.join(BINDING_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| LockError::Io {
            path: dir.clone(),
            source: e,
        })?;
        let path = Self::path(root);
        let yaml = serde_yaml_ng::to_string(self).map_err(|e| LockError::Yaml {
            path: path.clone(),
            source: e,
        })?;
        std::fs::write(&path, yaml).map_err(|e| LockError::Io { path, source: e })
    }
}

/// Sorted component ids resolved into the snapshot — human-readable detail.
/// The authoritative fingerprint is `Lock::snapshot_hash`; these lists just
/// make the resolution inspectable.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockComponents {
    #[serde(default)]
    pub rules: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub components: Vec<String>,
}

/// `.keel/keel.lock` — the pinned resolution (section 8.6).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Lock {
    pub project: String,
    pub workspace: String,
    #[serde(rename = "snapshotHash")]
    pub snapshot_hash: String,
    #[serde(rename = "keelVersion")]
    pub keel_version: String,
    pub components: LockComponents,
    /// The composition layers that contributed to the resolved snapshot
    /// (section 8.6: the lock pins the resolution), sorted and de-duplicated —
    /// e.g. `["global", "project:con-app"]`. Empty for a snapshot compiled
    /// without layer provenance. A layer appearing or disappearing is drift the
    /// snapshot hash already reflects; recording it keeps the resolution
    /// inspectable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub composition: Vec<String>,
}

impl Lock {
    pub fn path(root: &Path) -> PathBuf {
        root.join(BINDING_DIR).join(LOCK_FILE)
    }

    /// Builds the lock from a binding + the published snapshot. Deterministic:
    /// same binding + same snapshot ⇒ identical lock (invariant 9).
    pub fn generate(binding: &ProjectBinding, snapshot: &Snapshot, keel_version: &str) -> Self {
        let mut rules: Vec<String> = snapshot.rules.iter().map(|r| r.id.clone()).collect();
        rules.sort();
        // BTreeMap keys are already ordered, but collect explicitly for clarity.
        let tools: Vec<String> = snapshot.tools.keys().cloned().collect();
        let skills: Vec<String> = snapshot.skills.keys().cloned().collect();
        let agents: Vec<String> = snapshot.agents.keys().cloned().collect();
        let components: Vec<String> = snapshot.components.keys().cloned().collect();
        let mut composition: Vec<String> = snapshot
            .rules
            .iter()
            .filter_map(|r| r.origin_layer.clone())
            .collect();
        composition.sort();
        composition.dedup();
        Lock {
            project: binding.project.clone(),
            workspace: binding.workspace.clone(),
            snapshot_hash: snapshot.hash.to_string(),
            keel_version: keel_version.to_string(),
            components: LockComponents {
                rules,
                tools,
                skills,
                agents,
                components,
            },
            composition,
        }
    }

    pub fn load(root: &Path) -> Result<Self, LockError> {
        let path = Self::path(root);
        if !path.exists() {
            return Err(LockError::NoLock(path));
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| LockError::Io {
            path: path.clone(),
            source: e,
        })?;
        serde_yaml_ng::from_str(&raw).map_err(|e| LockError::Yaml { path, source: e })
    }

    pub fn write(&self, root: &Path) -> Result<(), LockError> {
        let dir = root.join(BINDING_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| LockError::Io {
            path: dir.clone(),
            source: e,
        })?;
        let path = Self::path(root);
        let yaml = serde_yaml_ng::to_string(self).map_err(|e| LockError::Yaml {
            path: path.clone(),
            source: e,
        })?;
        std::fs::write(&path, yaml).map_err(|e| LockError::Io { path, source: e })
    }

    /// Verifies a freshly-built resolution matches this lock. The compliance
    /// plane's core check (section 5.2): drift means the repo's pinned resolution no
    /// longer matches its workspace. Returns the human-readable reason on
    /// mismatch, `Ok(())` when they agree.
    pub fn verify(
        &self,
        binding: &ProjectBinding,
        snapshot: &Snapshot,
        keel_version: &str,
    ) -> Result<(), String> {
        let fresh = Lock::generate(binding, snapshot, keel_version);
        if fresh.snapshot_hash != self.snapshot_hash {
            return Err(format!(
                "snapshot hash drift: lock={} current={}",
                self.snapshot_hash, fresh.snapshot_hash
            ));
        }
        if fresh.project != self.project || fresh.workspace != self.workspace {
            return Err("binding drift: project/workspace differ from the lock".into());
        }
        if fresh.components != self.components {
            return Err("component drift: the resolved components differ from the lock".into());
        }
        if fresh.composition != self.composition {
            return Err(format!(
                "composition drift: resolved layers {:?} differ from the lock {:?}",
                fresh.composition, self.composition
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests-unit/lock.rs"]
mod tests;
