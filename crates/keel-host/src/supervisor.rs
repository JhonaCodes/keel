// SPDX-License-Identifier: Apache-2.0
//! Cognitive direction — the parent that helps without interfering (P3).
//!
//! Keel watches the live evidence ledger and, when a deterministic signal says
//! the model is stuck, records a suggestion for the OPERATOR. It does NOT
//! write into the model's input stream: steering the model's tokens directly
//! would interfere with its reasoning. It also MUST NOT write directly to the
//! controlling terminal while an interactive child owns it: concurrent output
//! can split ANSI/mouse-control sequences and corrupt a TUI. The launcher
//! presents queued notices only after the PTY and terminal mode are restored.
//!
//! The signal in v1 is OSCILLATION (spec section 6.5): the same rule blocking
//! at the same location three times in a session means the model lost or is
//! ignoring the context. Each distinct oscillation is surfaced once (no
//! nagging), rate-limited by a `seen` set.

use keel_engine::ledger::Ledger;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

const OSCILLATION_THRESHOLD: u64 = 3;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(750);

/// One deterministic key per surfaced signal, so the same oscillation is not
/// announced twice in a session.
fn key(rule_id: &str, file: Option<&str>, line: Option<u32>) -> String {
    format!("{rule_id}@{}:{}", file.unwrap_or("-"), line.unwrap_or(0))
}

/// Computes the suggestions that are NEW since the last poll for `session_id`,
/// recording them in `seen`. Pure with respect to I/O beyond the ledger read —
/// unit-tested directly.
pub fn new_suggestions(
    ledger: &Ledger,
    session_id: &str,
    seen: &mut BTreeSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    let oscillations = ledger
        .oscillations(OSCILLATION_THRESHOLD)
        .unwrap_or_default();
    for osc in oscillations {
        if osc.session_id.as_deref() != Some(session_id) {
            continue;
        }
        let k = key(&osc.rule_id, osc.file.as_deref(), osc.line);
        if !seen.insert(k) {
            continue;
        }
        let loc = match (&osc.file, osc.line) {
            (Some(f), Some(l)) => format!(" at {f}:{l}"),
            (Some(f), None) => format!(" at {f}"),
            _ => String::new(),
        };
        out.push(format!(
            "[keel] suggestion: `{}` has blocked {}x{loc} this session — the model may be \
             stuck. Consider intervening, or ask keel for the full skill (keel.skills.load).",
            osc.rule_id, osc.count
        ));
    }
    out
}

/// A supervisor running alongside an interactive client. Suggestions are
/// deliberately queued, never printed by the polling thread.
pub struct Supervisor {
    join: std::thread::JoinHandle<()>,
    notices: Receiver<String>,
}

impl Supervisor {
    /// Joins the worker after its shutdown signal and returns every queued
    /// suggestion. Calling this only after PTY teardown preserves the child's
    /// terminal byte stream as a single writer.
    pub fn finish(self) -> Vec<String> {
        let _ = self.join.join();
        self.notices.into_iter().collect()
    }
}

/// Spawns the supervisor loop against the workspace ledger. It polls until
/// `shutdown` flips and queues new suggestions for the launcher to render
/// after the interactive child exits.
pub fn spawn(ledger_path: PathBuf, session_id: String, shutdown: Arc<AtomicBool>) -> Supervisor {
    let (notices_tx, notices) = channel();
    let join = std::thread::spawn(move || {
        let mut seen = BTreeSet::new();
        while !shutdown.load(Ordering::SeqCst) {
            // A read may momentarily lose the SQLite lock to the broker's
            // write; that is fine — try again next tick, never crash the loop.
            if let Ok(ledger) = Ledger::open(&ledger_path) {
                for suggestion in new_suggestions(&ledger, &session_id, &mut seen) {
                    // The receiver is owned by the launcher. If it has gone
                    // away during shutdown there is nothing left to surface.
                    let _ = notices_tx.send(suggestion);
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    });
    Supervisor { join, notices }
}

#[cfg(test)]
#[path = "../tests-unit/supervisor.rs"]
mod tests;
