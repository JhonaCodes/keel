// SPDX-License-Identifier: Apache-2.0
//! Session Manager — minimal per-session state (spec section 8.1, section 6.5, future section 9).
//!
//! Tracks which skills a session has already received, implementing the
//! context economy ladder:
//!
//! ```text
//! nothing at start → full on normal load → compact only for explicit preview
//! ```
//!
//! A skill already loaded is NOT re-sent when the same momentum arises again
//! — the agent has it in context. `compact` is a catalog/preview variant; it is
//! not the authoritative instruction body. `full` is the real context a model
//! should follow when a skill matters. `deliver_skills` never re-sends what a
//! session already has at the requested level or higher.
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
/// `escalate` asks for the `full` variant. Normal MCP loading passes `true`;
/// explicit preview paths pass `false`. Oscillation can still pass `true`, but
/// it is no longer the primary route to full context.
///
/// | Situation | Delivery |
/// |---|---|
/// | not loaded, explicit preview | compact content + exemplar; mark compact |
/// | loaded compact, no escalation | one-line reference only (no re-send) |
/// | normal load / escalating, not at full yet | FULL content; mark full |
/// | loaded full | reference only |
pub fn deliver_skills(
    snapshot: &Snapshot,
    workspace_root: &Path,
    state: &mut SessionState,
    skill_refs: &[String],
    escalate: bool,
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

        let desired = if escalate {
            SkillLevel::Full
        } else {
            SkillLevel::Compact
        };
        let current = state.loaded_skills.get(&skill_id).copied();

        match current {
            Some(level) if level >= desired => {
                // Already in the agent's context: don't burn tokens re-sending
                // (the user's rule: only deliver a skill body once per level).
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
    let (label, content) = match (level, skill.full_content.as_ref(), &skill.full) {
        (SkillLevel::Full, Some(content), _) => ("full", content.clone()),
        (SkillLevel::Full, None, Some(full)) => (
            "full",
            std::fs::read_to_string(root.join(full)).unwrap_or_else(|_| {
                format!("(skill file `{full}` missing — see compile warnings)")
            }),
        ),
        // Escalation requested but no full variant exists: compact is the
        // best available — say so instead of silently downgrading.
        (SkillLevel::Full, None, None) => (
            "full-unavailable-using-compact",
            skill.compact_content.as_ref().cloned().unwrap_or_else(|| {
                std::fs::read_to_string(root.join(&skill.compact)).unwrap_or_else(|_| {
                    format!(
                        "(skill file `{}` missing — see compile warnings)",
                        skill.compact
                    )
                })
            }),
        ),
        (SkillLevel::Compact, _, _) => (
            "compact",
            skill.compact_content.as_ref().cloned().unwrap_or_else(|| {
                std::fs::read_to_string(root.join(&skill.compact)).unwrap_or_else(|_| {
                    format!(
                        "(skill file `{}` missing — see compile warnings)",
                        skill.compact
                    )
                })
            }),
        ),
    };

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
