// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `session` (relocated out of src; included via #[path] in src/session.rs).

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
            description: None,
            match_: Default::default(),
            version: "0.1.0".into(),
            compact: "skills/compact.md".into(),
            compact_content: None,
            full: Some("skills/full.md".into()),
            full_content: None,
            examples: vec![("bad()".into(), "good()".into())],
        },
    );
    Snapshot::build(vec![], BTreeMap::new(), skills, "t".into()).unwrap()
}

fn snapshot_with_inline_skill() -> Snapshot {
    let mut skills = BTreeMap::new();
    skills.insert(
        "inline-patterns".to_string(),
        CompiledSkill {
            id: "inline-patterns".into(),
            description: None,
            match_: Default::default(),
            version: "0.1.0".into(),
            compact: "<inline>".into(),
            compact_content: Some("USE the inline pattern.".into()),
            full: None,
            full_content: Some("FULL inline guide.".into()),
            examples: vec![],
        },
    );
    Snapshot::build(vec![], BTreeMap::new(), skills, "t".into()).unwrap()
}

fn tmp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("keel-session-test-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The L2 core still supports an explicit compact preview; the second preview
/// does NOT re-send; a full request upgrades the session.
#[test]
fn deliver_once_then_reference_then_escalate() {
    let root = tmp();
    let snap = snapshot_with_skill(&root);
    let mut state = SessionState::default();
    let refs = vec!["skill:access-patterns#compact".to_string()];

    // 1st explicit preview: compact content + exemplar.
    let p1 = deliver_skills(&snap, &root, &mut state, &refs, false, false);
    assert!(p1[0].contains("USE the approved pattern"));
    assert!(p1[0].contains("rejected: bad()"));
    assert!(p1[0].contains("accepted: good()"));

    // 2nd: already loaded → reference only, no content.
    let p2 = deliver_skills(&snap, &root, &mut state, &refs, false, false);
    assert!(p2[0].contains("already loaded"));
    assert!(!p2[0].contains("USE the approved pattern"));

    // Full request: upgrade to authoritative content even though compact was loaded.
    let p3 = deliver_skills(&snap, &root, &mut state, &refs, true, false);
    assert!(p3[0].contains("FULL guide"));

    // After full, another full request: reference only.
    let p4 = deliver_skills(&snap, &root, &mut state, &refs, true, false);
    assert!(p4[0].contains("already loaded"));

    std::fs::remove_dir_all(&root).ok();
}

/// section 10.4: a BLOCK carries the exemplar even when the skill body is already
/// loaded (force_exemplar). Otherwise a repeat block would ship "don't" without
/// the "do this" the model needs to correct.
#[test]
fn block_reattaches_exemplar_even_when_skill_already_loaded() {
    let root = tmp();
    let snap = snapshot_with_skill(&root);
    let mut state = SessionState::default();
    let refs = vec!["skill:access-patterns#compact".to_string()];

    // First delivery loads it (compact + exemplar).
    let _ = deliver_skills(&snap, &root, &mut state, &refs, false, false);
    // A blocking eval: already loaded, but the exemplar must be re-attached.
    let p = deliver_skills(&snap, &root, &mut state, &refs, false, true);
    assert!(
        p.iter().any(|c| c.contains("already loaded")),
        "skill body is not re-sent: {p:?}"
    );
    assert!(
        p.iter()
            .any(|c| c.contains("rejected: bad()") && c.contains("accepted: good()")),
        "a block must re-attach the exemplar even when the skill is loaded: {p:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn inline_skill_content_is_delivered_without_sidecar_files() {
    let root = tmp();
    let snap = snapshot_with_inline_skill();
    let mut state = SessionState::default();
    let refs = vec!["skill:inline-patterns#compact".to_string()];

    let compact = deliver_skills(&snap, &root, &mut state, &refs, false, false);
    assert!(compact[0].contains("USE the inline pattern."));

    let mut escalated = SessionState::default();
    let full = deliver_skills(&snap, &root, &mut escalated, &refs, true, false);
    assert!(full[0].contains("FULL inline guide."));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn store_roundtrips_state_per_session() {
    let root = tmp();
    let store = SessionStore::new(&root);
    let mut state = SessionState::default();
    state.loaded_skills.insert("x".into(), SkillLevel::Compact);
    store
        .save("sess/../1", &state, "2026-01-01T00:00:00Z")
        .unwrap();

    let back = store.load("sess/../1"); // same (sanitized) id → same file
    assert_eq!(back.loaded_skills.get("x"), Some(&SkillLevel::Compact));
    let other = store.load("other");
    assert!(other.loaded_skills.is_empty());
    std::fs::remove_dir_all(&root).ok();
}
