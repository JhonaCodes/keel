// SPDX-License-Identifier: Apache-2.0
//! ARCHITECTURE test: the plan's forbidden edges, verified in CI.
//!
//! The keel-engine modules live together in one crate (Phase 0 discipline),
//! so the Rust compiler cannot forbid imports between them. This test can:
//! if a PR introduces a forbidden edge, CI fails. Once the modules are
//! promoted to crates (Phase 1/2), these edges become compilation errors and
//! this test becomes redundant — until then, it is the lint.
//!
//! Edges and their rationale:
//! - runtime/snapshot/tools/ledger/sarif ⇏ keel_dsl — the runtime never sees
//!   authoring configuration (ADR-004): with no types to represent it, there
//!   is no path by which it can reach the model.
//! - compile ⇏ runtime — the compiler is a pure config→snapshot function;
//!   if it evaluated events, local and CI could diverge (invariant 9).
//! - ledger ⇏ runtime — the ledger is a sink: the runtime writes to it,
//!   never the other way around (evidence cannot become authoritative,
//!   invariant 16).

use std::path::Path;

fn src(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("could not read {path:?}: {e}"))
}

/// Looks for REAL uses (not comments) of a symbol in a source file.
fn uses_symbol(source: &str, symbol: &str) -> bool {
    source
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .any(|code| code.contains(symbol))
}

#[test]
fn runtime_side_modules_never_import_the_dsl() {
    for module in [
        "runtime.rs",
        "snapshot.rs",
        "tools.rs",
        "ledger.rs",
        "sarif.rs",
    ] {
        let source = src(module);
        assert!(
            !uses_symbol(&source, "keel_dsl"),
            "FORBIDDEN EDGE: {module} imports keel_dsl — the runtime side must \
             not know the authoring vocabulary (ADR-004 / invariant 9)"
        );
    }
}

#[test]
fn compiler_never_calls_the_runtime() {
    let source = src("compile.rs");
    for forbidden in ["crate::runtime", "use crate::runtime", "runtime::evaluate"] {
        assert!(
            !uses_symbol(&source, forbidden),
            "FORBIDDEN EDGE: compile.rs references `{forbidden}` — the \
             compiler is pure config→snapshot (compiler ⇏ runtime)"
        );
    }
}

#[test]
fn ledger_never_calls_the_runtime() {
    let source = src("ledger.rs");
    assert!(
        !uses_symbol(&source, "crate::runtime"),
        "FORBIDDEN EDGE: ledger.rs references the runtime — the ledger is a \
         sink (invariant 16)"
    );
}

/// The ledger is APPEND-ONLY via API surface: no UPDATE or DELETE statement
/// over the evidence table (invariant 16).
#[test]
fn ledger_has_no_update_or_delete_on_evidence() {
    let source = src("ledger.rs").to_uppercase();
    assert!(
        !source.contains("UPDATE EVIDENCE"),
        "the ledger must not update evidence: it is append-only"
    );
    assert!(
        !source.contains("DELETE FROM EVIDENCE"),
        "the ledger must not delete evidence: it is append-only"
    );
}

/// Phase 0: no code path with a model SDK, HTTP, or an async runtime.
#[test]
fn no_llm_no_http_no_tokio_in_phase_0() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    for forbidden in ["tokio", "reqwest", "hyper", "anthropic", "openai"] {
        assert!(
            !manifest.contains(forbidden),
            "passive Phase 0 does not admit `{forbidden}` (ADR-021: ledger \
             first, no semantic evaluators until Phase 2)"
        );
    }
}
