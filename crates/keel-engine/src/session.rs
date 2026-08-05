// SPDX-License-Identifier: Apache-2.0
//! Session Manager — minimal per-session state (spec §8.1, §6.5, future §9).
//!
//! Tracks which skills a session has already received, implementing the
//! context economy ladder:
//!
//! ```text
//! nothing at start → compact on first activation → full only on oscillation
//! ```
//!
//! A skill already loaded is NOT re-sent when the same momentum arises again
//! — the agent has it in context. The exception is oscillation (§6.5): three
//! repeated findings at the same rule+location mean the agent lost or is
//! ignoring the context, so the runtime escalates to the `full` variant.
//!
//! INVARIANT 16: this state is append-only in spirit — it only ever records
//! what was delivered; it cannot touch enforcement, scope, validation or
//! executors of any rule.
//!
//! BOUNDARY RULE: does not import `keel_dsl` — operates on compiled skills.

use crate::snapshot::{CompiledSkill, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Loading level a session has received for a skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillLevel {
    Compact,
    Full,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionState {
    #[serde(default)]
    pub loaded_skills: BTreeMap<String, SkillLevel>,
    #[serde(default)]
    pub updated_at: String,
}

pub struct SessionStore {
    dir: PathBuf,
}

impl SessionStore {
    /// `state_dir` is the workspace's `.keel-state/`; sessions live under
    /// `sessions/<id>.json` inside it.
    pub fn new(state_dir: &Path) -> Self {
        SessionStore {
            dir: state_dir.join("sessions"),
        }
    }

    fn path_for(&self, session_id: &str) -> PathBuf {
        // Sanitize: session ids come from external clients (hook payloads).
        let safe: String = session_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        self.dir.join(format!("{safe}.json"))
    }

    pub fn load(&self, session_id: &str) -> SessionState {
        std::fs::read_to_string(self.path_for(session_id))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, session_id: &str, state: &SessionState, now: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let mut state = state.clone();
        state.updated_at = now.to_string();
        std::fs::write(
            self.path_for(session_id),
            serde_json::to_string_pretty(&state)?,
        )
    }
}

/// Resolves what a rule's `load.skills` should DELIVER to the session right
/// now, updating the state. Returns the payload chunks for the packet.
///
/// | Situation | Delivery |
/// |---|---|
/// | not loaded, no oscillation | compact content + exemplar; mark compact |
/// | loaded compact, no oscillation | one-line reference only (no re-send) |
/// | oscillating, not at full yet | FULL content (escalation §6.5); mark full |
/// | loaded full | reference only |
pub fn deliver_skills(
    snapshot: &Snapshot,
    workspace_root: &Path,
    state: &mut SessionState,
    skill_refs: &[String],
    oscillating: bool,
) -> Vec<String> {
    let mut payload = Vec::new();

    for skill_ref in skill_refs {
        let skill_id = skill_ref
            .strip_prefix("skill:")
            .unwrap_or(skill_ref)
            .split('#')
            .next()
            .unwrap_or_default()
            .to_string();

        let Some(skill) = snapshot.skills.get(&skill_id) else {
            // Compile-time resolution makes this unreachable for compiled
            // rules; stay fail-safe anyway (a missing skill must not panic).
            payload.push(format!("skill {skill_id}: not in snapshot"));
            continue;
        };

        let desired = if oscillating { SkillLevel::Full } else { SkillLevel::Compact };
        let current = state.loaded_skills.get(&skill_id).copied();

        match current {
            Some(level) if level >= desired => {
                // Already in the agent's context: don't burn tokens re-sending
                // (the user's rule: only re-deliver when context is lost —
                // which surfaces as oscillation and is handled below).
                payload.push(format!("skill {skill_id} already loaded ({level:?})"));
            }
            _ => {
                let chunk = render_skill(skill, workspace_root, desired);
                payload.push(chunk);
                state.loaded_skills.insert(skill_id, desired);
            }
        }
    }
    payload
}

fn render_skill(skill: &CompiledSkill, root: &Path, level: SkillLevel) -> String {
    let (label, path) = match (level, &skill.full) {
        (SkillLevel::Full, Some(full)) => ("full", full.as_str()),
        // Escalation requested but no full variant exists: compact is the
        // best available — say so instead of silently downgrading.
        (SkillLevel::Full, None) => ("full-unavailable-using-compact", skill.compact.as_str()),
        (SkillLevel::Compact, _) => ("compact", skill.compact.as_str()),
    };

    let content = std::fs::read_to_string(root.join(path))
        .unwrap_or_else(|_| format!("(skill file `{path}` missing — see compile warnings)"));

    let mut out = format!("--- skill {} ({label}) ---\n{}", skill.id, content.trim_end());
    // Exemplar pair (§10.4): mandatory companion of a block whenever the
    // skill provides pairs — the difference between "don't" and "do this".
    if let Some((rejected, accepted)) = skill.examples.first() {
        out.push_str(&format!("\nrejected: {rejected}\naccepted: {accepted}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn snapshot_with_skill(root: &Path) -> Snapshot {
        std::fs::create_dir_all(root.join("skills")).unwrap();
        std::fs::write(root.join("skills/compact.md"), "USE the approved pattern.").unwrap();
        std::fs::write(root.join("skills/full.md"), "FULL guide with all cases.").unwrap();

        let mut skills = BTreeMap::new();
        skills.insert(
            "access-patterns".to_string(),
            CompiledSkill {
                id: "access-patterns".into(),
                compact: "skills/compact.md".into(),
                full: Some("skills/full.md".into()),
                examples: vec![("bad()".into(), "good()".into())],
            },
        );
        Snapshot::build(vec![], BTreeMap::new(), skills, "t".into()).unwrap()
    }

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("keel-session-test-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The L2 core: first activation delivers compact+exemplar; the second
    /// does NOT re-send; oscillation escalates to full.
    #[test]
    fn deliver_once_then_reference_then_escalate() {
        let root = tmp();
        let snap = snapshot_with_skill(&root);
        let mut state = SessionState::default();
        let refs = vec!["skill:access-patterns#compact".to_string()];

        // 1st: full compact content + exemplar.
        let p1 = deliver_skills(&snap, &root, &mut state, &refs, false);
        assert!(p1[0].contains("USE the approved pattern"));
        assert!(p1[0].contains("rejected: bad()"));
        assert!(p1[0].contains("accepted: good()"));

        // 2nd: already loaded → reference only, no content.
        let p2 = deliver_skills(&snap, &root, &mut state, &refs, false);
        assert!(p2[0].contains("already loaded"));
        assert!(!p2[0].contains("USE the approved pattern"));

        // Oscillation: escalate to FULL even though compact was loaded.
        let p3 = deliver_skills(&snap, &root, &mut state, &refs, true);
        assert!(p3[0].contains("FULL guide"));

        // After full, even oscillating: reference only.
        let p4 = deliver_skills(&snap, &root, &mut state, &refs, true);
        assert!(p4[0].contains("already loaded"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn store_roundtrips_state_per_session() {
        let root = tmp();
        let store = SessionStore::new(&root);
        let mut state = SessionState::default();
        state.loaded_skills.insert("x".into(), SkillLevel::Compact);
        store.save("sess/../1", &state, "2026-01-01T00:00:00Z").unwrap();

        let back = store.load("sess/../1"); // same (sanitized) id → same file
        assert_eq!(back.loaded_skills.get("x"), Some(&SkillLevel::Compact));
        let other = store.load("other");
        assert!(other.loaded_skills.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}