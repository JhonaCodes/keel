// SPDX-License-Identifier: Apache-2.0
//! keel-tests — the workspace's top-level integration-test crate. The tests
//! themselves live under `test/tests/`.
//!
//! It also hosts the Phase 0c enforcement-measurement harness (`measure`,
//! spec section 15.1), exposed as a library so both the `keel-measure` binary
//! and the harness integration test drive the same code path.

pub mod measure;
