// SPDX-License-Identifier: Apache-2.0
//! Environment constraints (section 11.4, F1): `constraints.environment.allow/deny`
//! used to be compiled and hashed but NEVER evaluated (the one hidden
//! functional partial). These end-to-end tests, through the real `keel` binary,
//! prove it now enforces: `deny` blocks always, a non-empty `allow` is a strict
//! allowlist, and the block flows through the rule's `invalid` branch (packet
//! included). Run hermetically so the host env cannot interfere.

use keel_tests::hermetic::HermeticWs;

/// A rule whose ONLY gate is the environment constraint. `validate` never
/// matches, so a clean environment resolves `valid` (allow) and only the
/// constraint can block — isolating the F1 behavior.
const ENV_RULE: &str = r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: db.env, version: 1.0.0, author: test, adrRef: adr:ADR-001, reviewAfter: P6M }
spec:
  reversibility: irreversible
  on: [command.requested]
  validate: { using: builtin:text.contains, with: { value: "NEVER_MATCH" } }
  enforcement:
    invalid: { decision: block, report: { message: "environment not permitted" } }
    unknown: { decision: deny-pending-approval }
    valid:   { decision: allow }
  constraints:
    environment:
      allow: [local, docker-dev]
      deny:  [staging, production]
"#;

fn cmd(conn: &str) -> String {
    format!(
        r#"{{"kind":"command.requested","command":"psql {conn}","content":"SELECT 1","session_id":"c"}}"#
    )
}

/// `deny` blocks always: a connection to a denied environment is prevented
/// (inner ring, exit 2) and the packet names the denied environment.
#[test]
fn denied_environment_is_blocked_with_reason() {
    let ws = HermeticWs::new(&[("env.yaml", ENV_RULE)]);
    let (code, packet) = ws.gate_output("c", &cmd("postgresql://u@production-db/app"), &[], true);
    assert_eq!(code, 2, "a denied environment must be blocked (exit 2)");
    assert!(
        packet.contains("denied environment") && packet.contains("production"),
        "the packet must name the denied environment: {packet}"
    );
}

/// An allowed environment passes the constraint; `validate` then resolves
/// `valid` → allow (exit 0). Proves the constraint does not over-block.
#[test]
fn allowed_environment_passes() {
    let ws = HermeticWs::new(&[("env.yaml", ENV_RULE)]);
    let code = ws.gate("c", &cmd("postgresql://u@local/app"), &[], true);
    assert_eq!(code, 0, "an allowed environment must pass (exit 0)");
}

/// A non-empty `allow` is a strict allowlist: a command that names NO allowed
/// environment is denied even though it matches nothing in `deny`.
#[test]
fn unlisted_environment_fails_the_allowlist() {
    let ws = HermeticWs::new(&[("env.yaml", ENV_RULE)]);
    let (code, packet) = ws.gate_output("c", &cmd("somewhere/app"), &[], true);
    assert_eq!(
        code, 2,
        "no allowed environment present must be denied (exit 2)"
    );
    assert!(
        packet.contains("no allowed environment"),
        "the packet must explain the allowlist miss: {packet}"
    );
}
