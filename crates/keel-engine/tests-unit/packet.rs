// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `packet` (relocated out of src; included via #[path] in src/packet.rs).

use super::*;
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
fn blocked_packet_carries_constraint_and_evidence() {
    let p = render(&eval(Decision::Block), "ev_1", &[]);
    assert!(p.starts_with("BLOCKED (db.gate-sql-execution)"));
    assert!(p.contains("DROP is destructive"));
    assert!(p.contains("Evidence: ev_1 logged"));
}

/// section 4.7: the deny-pending packet tells the model NOT to retry — a human
/// decides, never the model.
#[test]
fn deny_pending_packet_escalates_to_human() {
    let p = render(&eval(Decision::DenyPendingApproval), "ev_2", &[]);
    assert!(p.contains("pending approval"));
    assert!(p.contains("human approval"));
}
