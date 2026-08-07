// SPDX-License-Identifier: Apache-2.0
//! Session Manager — minimal per-session state (spec section 8.1, section 6.5, future section 9).
//!
//! Tracks which skills a session has already received, implementing the
//! context economy ladder:
//!
//! ```text
//! nothing at start → compact on first activation → full only on oscillation
//! ```
//!
//! A skill already loaded is NOT re-sent when the same momentum arises again
//! — the agent has it in context. The exception is oscillation (section 6.5): three
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
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
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
/// | oscillating, not at full yet | FULL content (escalation section 6.5); mark full |
/// | loaded full | reference only |
pub fn deliver_skills(
    snapshot: &Snapshot,
    workspace_root: &Path,
    state: &mut SessionState,
    skill_refs: &[String],
    oscillating: bool,
    force_exemplar: bool,
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

        let desired = if oscillating {
            SkillLevel::Full
        } else {
            SkillLevel::Compact
        };
        let current = state.loaded_skills.get(&skill_id).copied();

        match current {
            Some(level) if level >= desired => {
                // Already in the agent's context: don't burn tokens re-sending
                // (the user's rule: only re-deliver when context is lost —
                // which surfaces as oscillation and is handled below).
                payload.push(format!("skill {skill_id} already loaded ({level:?})"));
                // section 10.4: a BLOCK packet must still carry the exemplar even
                // when the skill body is already loaded — the rejected/accepted
                // pair is the difference between "don't" and "do this".
                if force_exemplar && let Some((rejected, accepted)) = skill.examples.first() {
                    payload.push(format!("rejected: {rejected}\naccepted: {accepted}"));
                }
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

    let mut out = format!(
        "--- skill {} ({label}) ---\n{}",
        skill.id,
        content.trim_end()
    );
    // Exemplar pair (section 10.4): mandatory companion of a block whenever the
    // skill provides pairs — the difference between "don't" and "do this".
    if let Some((rejected, accepted)) = skill.examples.first() {
        out.push_str(&format!("\nrejected: {rejected}\naccepted: {accepted}"));
    }
    out
}

#[cfg(test)]
#[path = "../tests-unit/session.rs"]
mod tests;
