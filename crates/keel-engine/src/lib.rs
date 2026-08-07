// SPDX-License-Identifier: Apache-2.0
//! keel-engine — Keel's compiler, snapshot, tools, ledger and runtime.
//!
//! The five components live as MODULES of one crate (Phase 0 discipline:
//! no pre-fragmenting without reuse pressure, ADR-020), but with the same
//! forbidden edges they would have as crates — verified by the architecture
//! test `tests/arch_boundaries.rs`:
//!
//! ```text
//! compile   → keel-dsl + snapshot        (config → artifact; PURE)
//! composition → snapshot + keel-dsl       (folds layers + verifies locked
//!                                          monotonicity, section 7.4; PURE,
//!                                          compile-side)
//! snapshot  → keel-core                  (⇏ dsl: the compiled artifact does
//!                                          not drag authoring vocabulary
//!                                          along)
//! tools     → keel-core                  (⇏ dsl)
//! ledger    → keel-core                  (⇏ dsl, ⇏ runtime: it is a sink)
//! resolution → dsl + workspace + lock    (config-side, pre-compile: selects
//!                                          which layers apply by repo identity,
//!                                          section 7.1 — like compile, it may
//!                                          see the DSL; it is not runtime-side)
//! runtime   → snapshot + tools           (⇏ dsl: the runtime NEVER sees
//!                                          configuration, only the snapshot —
//!                                          structural guarantee of ADR-004)
//! testkit   → runtime + keel-dsl         (orchestration of the test gate;
//!                                          compile does NOT call testkit —
//!                                          the CLI orchestrates: compile →
//!                                          test → publish, section 10.2)
//! packet    → runtime + keel-core        (⇏ dsl: rendered from compiled
//!                                          artifacts only, ADR-004)
//! adapter   → snapshot + keel-core       (⇏ dsl: launch containment
//!                                          preflight over the snapshot,
//!                                          invariant 8)
//! ```
//!
//! WHY: if the runtime could import the DSL, there would be a path by which
//! configuration could reach the model; if the compiler knew about the
//! runtime, it would stop being a pure config→snapshot function and local/CI
//! could diverge (invariant 9).

pub mod adapter;
pub mod compile;
pub mod composition;
pub mod ledger;
pub mod lock;
pub mod packet;
pub mod resolution;
pub mod runtime;
pub mod sarif;
pub mod snapshot;
pub mod testkit;
pub mod tools;
pub mod workspace;
