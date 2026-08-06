// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `rule` (relocated out of src; included via #[path] in src/rule.rs).

use super::*;

#[test]
fn tool_ref_parses_both_prefixes_and_roundtrips() {
    let b: ToolRef = "builtin:text.contains".parse().unwrap();
    assert_eq!(b, ToolRef::Builtin("text.contains".into()));
    assert_eq!(b.to_string(), "builtin:text.contains");

    let t: ToolRef = "tool:sqlglot.classify-statement".parse().unwrap();
    assert_eq!(t, ToolRef::External("sqlglot.classify-statement".into()));

    assert!("mcp:whatever".parse::<ToolRef>().is_err());
}

#[test]
fn onfail_deny_is_certain_denial_not_uncertainty() {
    // deny = certainty of violation → block. deny-pending-approval is
    // reserved for `unknown` on irreversibles (section 4.7).
    assert_eq!(OnFail::Deny.as_declared_decision(), Decision::Block);
    assert_eq!(OnFail::Review.as_declared_decision(), Decision::Review);
}
