// SPDX-License-Identifier: Apache-2.0
//! keel-engine — Keel's compiler, snapshot, tools, ledger and runtime.
//!
//! The five components live as MODULES of one crate (Phase 0 discipline:
//! no pre-fragmenting without reuse pressure, ADR-020), but with the same
//! forbidden edges they would have as crates — verified by the architecture
//! test `tests/arch_boundaries.rs`:
//!
//! ```text
//! audit     → ledger (keel-core)        (§14: runs an executor, files origin=semantic)
//! compile   → keel-dsl + snapshot        (config → artifact; PURE)
//! snapshot  → keel-core                  (⇏ dsl: the compiled artifact does
//!                                          not drag authoring vocabulary
//!                                          along)
//! tools     → keel-core                  (⇏ dsl)
//! ledger    → keel-core                  (⇏ dsl, ⇏ runtime: it is a sink)
//! runtime   → snapshot + tools           (⇏ dsl: the runtime NEVER sees
//!                                          configuration, only the snapshot —
//!                                          structural guarantee of ADR-004)
//! session   → snapshot (keel-core)      (⇏ dsl: delivers compiled skills)
//! testkit   → runtime + keel-dsl         (orchestration of the test gate;
//!                                          compile does NOT call testkit —
//!                                          the CLI orchestrates: compile →
//!                                          test → publish, §10.2)
//! ```
//!
//! WHY: if the runtime could import the DSL, there would be a path by which
//! configuration could reach the model; if the compiler knew about the
//! runtime, it would stop being a pure config→snapshot function and local/CI
//! could diverge (invariant 9).

pub mod audit;
pub mod compile;
pub mod ledger;
pub mod lock;
pub mod packet;
pub mod runtime;
pub mod sarif;
pub mod session;
pub mod snapshot;
pub mod testkit;
pub mod tools;
pub mod workspace;