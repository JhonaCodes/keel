// SPDX-License-Identifier: Apache-2.0
//! Test isolation (#12): a test's result must depend ONLY on what the test
//! declares, never on the host/CI environment. Proves the two mechanisms that
//! guarantee it: `env_clear()` in the hermetic harness (the host cannot supply
//! a var) and `keel gate --no-inherit-env` (the gate evaluates preconditions
//! against the event's env only).
//!
//! The rule below denies an irreversible `command.requested` unless
//! `KEEL_ISOLATION_SECRET` is present in the environment at request time
//! (ADR-022). That single precondition makes env contamination observable as
//! an exit code: pass (0) when the var is seen, deny (2) when it is not.

use keel_tests::hermetic::HermeticWs;

const ENV_GATE_RULE: &str = r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: iso.env-gate, version: 1.0.0, author: test, adrRef: adr:ADR-001, reviewAfter: P6M }
spec:
  reversibility: irreversible
  on: [command.requested]
  preconditions:
    - using: builtin:env.present
      with: { name: KEEL_ISOLATION_SECRET }
      onFail: deny
  validate: { using: builtin:text.contains, with: { value: "NEVER_MATCH" } }
  enforcement:
    invalid: { decision: block }
    unknown: { decision: deny-pending-approval }
    valid:   { decision: allow }
"#;

const CLEAN_CMD: &str =
    r#"{"kind":"command.requested","command":"echo hi","content":"clean","session_id":"iso"}"#;

/// The host cannot supply the precondition's var: with a cleared child
/// environment and nothing injected, the precondition FAILS (exit 2) — no
/// matter what the developer or CI happens to have exported. If `env_clear`
/// leaked the host env and the host had `KEEL_ISOLATION_SECRET`, this would
/// flip to 0; it does not.
#[test]
fn host_env_cannot_satisfy_a_precondition() {
    let ws = HermeticWs::new(&[("env-gate.yaml", ENV_GATE_RULE)]);
    let code = ws.gate("iso", CLEAN_CMD, &[], false);
    assert_eq!(
        code, 2,
        "with no var declared, the precondition must fail regardless of host env"
    );
}

/// The contamination path AND its fix, in one controlled experiment. With the
/// var injected into the child env, the live-env default (ADR-022) folds it in
/// and the precondition PASSES (exit 0). The exact same run with
/// `--no-inherit-env` ignores the ambient env and the precondition FAILS
/// (exit 2). Same inputs, the flag is the only difference — proving a host var
/// can silently flip a verdict, and that the flag stops it.
#[test]
fn injected_var_leaks_by_default_but_not_with_no_inherit_env() {
    let ws = HermeticWs::new(&[("env-gate.yaml", ENV_GATE_RULE)]);
    let leaked = ws.gate("iso", CLEAN_CMD, &[("KEEL_ISOLATION_SECRET", "1")], false);
    assert_eq!(
        leaked, 0,
        "the live-env default folds the ambient var in — precondition passes"
    );
    let sealed = ws.gate("iso", CLEAN_CMD, &[("KEEL_ISOLATION_SECRET", "1")], true);
    assert_eq!(
        sealed, 2,
        "--no-inherit-env evaluates only the event's env — precondition fails"
    );
}

/// A var carried by the EVENT itself is honored under `--no-inherit-env` (it is
/// event state, not host state) — the flag suppresses ambient inheritance, not
/// the event's own declared world.
#[test]
fn event_declared_env_is_honored_even_with_no_inherit_env() {
    let ws = HermeticWs::new(&[("env-gate.yaml", ENV_GATE_RULE)]);
    let event = r#"{"kind":"command.requested","command":"echo hi","content":"clean","session_id":"iso","env":{"KEEL_ISOLATION_SECRET":"1"}}"#;
    let code = ws.gate("iso", event, &[], true);
    assert_eq!(
        code, 0,
        "the event's own env satisfies the precondition even with the flag"
    );
}
