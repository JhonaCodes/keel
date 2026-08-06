// SPDX-License-Identifier: Apache-2.0
//! Context-compaction re-check (#7, section 6.5). The L2 rule is "don't re-send a
//! skill already loaded" — UNLESS the model lost it. Until now the only
//! re-delivery trigger was oscillation (a same-location retry proxy); there was
//! no signal for an actual context compaction. `context.compacted` is that
//! signal: it resets the session's delivered-skill state so the next matching
//! rule re-delivers. Verified through the real binary against the session file.

use keel_tests::hermetic::HermeticWs;

// A minimal valid rule so the workspace compiles; `context.compacted` never
// reaches rule evaluation (it is handled and returned before the snapshot load).
const NOOP_RULE: &str = r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: r.noop, version: 1.0.0, author: test, adrRef: adr:ADR-001, reviewAfter: P6M }
spec:
  reversibility: reversible
  on: [file.edited]
  detect:   { using: builtin:text.contains, with: { value: "ZZZ" } }
  validate: { using: builtin:text.contains, with: { value: "ZZZ" } }
  enforcement:
    invalid: { decision: review }
    valid:   { decision: allow }
"#;

/// A `context.compacted` event clears the session's delivered-skill state (so a
/// skill the model lost gets re-delivered on the next match) and exits 0.
#[test]
fn context_compacted_resets_delivered_skill_state() {
    let ws = HermeticWs::new(&[("noop.yaml", NOOP_RULE)]);

    // Seed a session that already has a skill marked loaded.
    let sessions = ws.as_ref().join(".keel-state").join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let session_file = sessions.join("s.json");
    std::fs::write(
        &session_file,
        r#"{"loaded_skills":{"access-patterns":"Compact"},"updated_at":"2026-01-01T00:00:00Z"}"#,
    )
    .unwrap();

    let code = ws.gate(
        "s",
        r#"{"kind":"context.compacted","session_id":"s"}"#,
        &[],
        true,
    );
    assert_eq!(code, 0, "context.compacted governs no action → exit 0");

    let after = std::fs::read_to_string(&session_file).unwrap();
    assert!(
        !after.contains("access-patterns"),
        "the loaded-skill state must be cleared after context.compacted: {after}"
    );
}
