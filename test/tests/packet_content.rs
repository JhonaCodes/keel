// SPDX-License-Identifier: Apache-2.0
//! ContextPacket content through the REAL binary (section 10.4). Until now the
//! black-box tests asserted only exit codes; the packet the model actually
//! receives on stderr was never checked end-to-end. This asserts the packet
//! carries the verdict, the finding, the `source` (rule + immutable snapshot
//! hash) and the evidence id — the fields the spec requires the model to see.

use keel_tests::hermetic::HermeticWs;

const DROP_RULE: &str = r#"apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: db.no-drop, version: 1.0.0, author: test, adrRef: adr:ADR-001, reviewAfter: P6M }
spec:
  reversibility: irreversible
  on: [command.requested]
  detect:   { using: builtin:text.contains, with: { value: "DROP" } }
  validate: { using: builtin:text.contains, with: { value: "DROP" } }
  enforcement:
    invalid: { decision: block, report: { message: "no DROP allowed" } }
    unknown: { decision: deny-pending-approval }
    valid:   { decision: allow }
"#;

/// A blocked inner-ring command emits, on stderr: the BLOCKED header, the
/// finding, and a `source` line pinning the rule id and the snapshot hash
/// (section 10.4). Run hermetically so the assertion is host-independent.
#[test]
fn blocked_packet_on_stderr_carries_source_and_snapshot() {
    let ws = HermeticWs::new(&[("drop.yaml", DROP_RULE)]);
    let event = r#"{"kind":"command.requested","command":"psql -c stmt","content":"DROP DATABASE prod","session_id":"s"}"#;
    let (code, packet) = ws.gate_output("s", event, &[], true);

    assert_eq!(
        code, 2,
        "a violating inner-ring command must block (exit 2)"
    );
    assert!(
        packet.contains("BLOCKED (db.no-drop)"),
        "packet must state the blocking verdict + rule: {packet}"
    );
    assert!(
        packet.contains("no DROP allowed"),
        "packet must carry the finding/required action: {packet}"
    );
    assert!(
        packet.contains("source: rule=db.no-drop snapshot=sha256:"),
        "packet must pin its source rule and immutable snapshot (section 10.4): {packet}"
    );
    assert!(
        packet.contains("Evidence:") && packet.contains("logged"),
        "packet must carry the evidence id: {packet}"
    );
}

/// The other half of the contract: a clean command is allowed (exit 0) and
/// emits no blocking packet.
#[test]
fn clean_command_emits_no_block() {
    let ws = HermeticWs::new(&[("drop.yaml", DROP_RULE)]);
    let event = r#"{"kind":"command.requested","command":"psql -c stmt","content":"SELECT 1","session_id":"s"}"#;
    let (code, _packet) = ws.gate_output("s", event, &[], true);
    assert_eq!(code, 0, "a clean command must be allowed (exit 0)");
}
