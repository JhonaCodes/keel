// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `lib` (relocated out of src; included via #[path] in src/lib.rs).

    use super::*;

    /// The §7.4-D3 lattice is load-bearing: passive forcing and the future
    /// D3 check depend on this exact ordering.
    #[test]
    fn decision_lattice_order() {
        assert!(Decision::Allow < Decision::Review);
        assert!(Decision::Review < Decision::Block);
        assert!(Decision::Block < Decision::DenyPendingApproval);
    }

    #[test]
    fn decision_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_string(&Decision::DenyPendingApproval).unwrap(),
            "\"deny-pending-approval\""
        );
    }

    #[test]
    fn verdict_and_origin_serialize_lowercase() {
        assert_eq!(serde_json::to_string(&Verdict::Unknown).unwrap(), "\"unknown\"");
        assert_eq!(
            serde_json::to_string(&OriginClass::Deterministic).unwrap(),
            "\"deterministic\""
        );
    }
