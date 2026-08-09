// SPDX-License-Identifier: Apache-2.0
//! keel-host — the parent runtime (spec section 5.3, section 12).
//!
//! `keel <cli>` launches the client CLI as a CHILD process under a PTY, inside
//! an environment keel fabricates. The model never configures this layer and
//! cannot remove it: it is not client configuration — it is the environment
//! the child is born into.
//!
//! Planes (enforcement NEVER depends on the model's cooperation):
//! - Containment (this crate, F1): governed commands are interposed via a
//!   session shim dir prepended to PATH; each shim forwards argv to the
//!   broker over a UNIX socket; the broker evaluates `command.requested` in
//!   `Enforce` mode — a blocked command never exists as a process.
//! - OS sandbox (F2): the kernel-level backstop (Seatbelt/Landlock).
//! - Convergence (F3): the keel MCP endpoint for skills/agents.
//! - Supervision (F5): PTY tee + live ledger → visible suggestions.
//!
//! BOUNDARY RULE: does not import `keel_dsl` — the host works from the
//! compiled snapshot only (ADR-004).

pub mod broker;
pub mod config;
pub mod dotenv;
pub mod launch;
pub mod mcp;
pub mod pty;
pub mod sandbox;
pub mod shims;
pub mod supervisor;

pub use launch::{ContainmentMode, LaunchOptions, launch};
