// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `packet` (relocated out of src; included via #[path] in src/packet.rs).

use super::*;
use crate::runtime::Evaluation;
use crate::tools::Finding;
use keel_core::{OriginClass, Verdict};

fn eval(effective: Decision) -> Evaluation {
    Evaluation {
        rule_id: "db.gate-sql-execution".into(),
        rule_version: Some("1.0.0".into()),
        verdict: Verdict::Invalid,
        origin: OriginClass::Deterministic,
        declared_decision: Decision::Block,
        effective_decision: effective,
        latency_ms: 3,
        tokens: 0,
        findings: vec![Finding {
            message: "DROP is destructive".into(),
            file: None,
            line: None,
        }],
        detail: None,
        load_skills: vec![],
    }
}

#[test]
fn blocked_packet_carries_constraint_evidence_and_source() {
    let p = render(&eval(Decision::Block), "ev_1", &[], "sha256:abc123");
    assert!(p.contains("BLOCKED (db.gate-sql-execution)"));
    assert!(p.contains("DROP is destructive"));
    assert!(p.contains("Evidence: ev_1 logged"));
    // section 10.4 `source`: pins the packet to the rule + immutable snapshot.
    assert!(
        p.contains("source: rule=db.gate-sql-execution snapshot=sha256:abc123"),
        "the packet must name its source rule and snapshot: {p}"
    );
}

/// section 4.7: the deny-pending packet tells the model NOT to retry — a human
/// decides, never the model.
#[test]
fn deny_pending_packet_escalates_to_human() {
    let p = render(
        &eval(Decision::DenyPendingApproval),
        "ev_2",
        &[],
        "sha256:x",
    );
    assert!(p.contains("pending approval"));
    assert!(p.contains("human approval"));
}
