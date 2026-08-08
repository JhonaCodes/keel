// SPDX-License-Identifier: Apache-2.0
//! `KnowledgeChain` — hash-chained, append-only growth for a `Knowledge`
//! component (spec-adjacent, ADR pending).
//!
//! WHY THIS EXISTS: the snapshot hash (`ContentHash::of_canonical` over
//! `HashableContent`, keel-engine) is a single aggregate over ALL authored
//! resources — correct for config that changes by deliberate authoring and
//! review, wrong for content that grows constantly as a normal part of
//! operating (a session's accumulated memory). Using `spec.content` (a path)
//! for a `Knowledge` component already keeps the snapshot hash stable across
//! growth (only the path string is hashed, not file bytes — the same
//! treatment Skills get). What growth still needs, and what this module
//! provides, is an INTEGRITY guarantee distinct from drift detection: not
//! "does this match a frozen state" but "was any past entry rewritten after
//! the fact".
//!
//! MECHANISM: each entry's hash covers its own content AND the previous
//! entry's hash (`entry_hash = ContentHash::of_canonical(component_id, seq,
//! content, prev_hash)`), the same shape as a Git commit chain or a Merkle
//! log. `verify_chain` walks the chain and recomputes every hash from
//! storage — it never trusts the stored `entry_hash` at face value. This
//! catches both a single tampered entry (recomputed hash mismatches the
//! stored one) and a tampered entry whose OWN hash was forged to match
//! (the NEXT entry's `prev_hash` link no longer resolves). A fully
//! self-consistent rewrite of an entire suffix of the chain is NOT
//! detectable by this module alone — that requires an external witness
//! (a checkpoint anchored in `.keel/keel.lock`, versioned in git and
//! compared by CI), which is a separate, later piece.
//!
//! CONVENTIONS (matching `keel-engine::ledger` and this crate's own
//! `store`/`scheduler`): a private `Connection` per component, `open`/
//! `open_in_memory` constructors, `CREATE TABLE IF NOT EXISTS` in a private
//! `initialize`, `thiserror::Error` with `#[from] rusqlite::Error`, ULID ids,
//! RFC3339 timestamps via `jiff`. APPEND-ONLY by API surface: no UPDATE or
//! DELETE method is exposed over `knowledge_entries`.
//!
//! BOUNDARY: this module imports `keel_core` only (for `ContentHash`), never
//! the authoring DSL crate — same discipline as `keel-engine::ledger`.

use keel_core::ContentHash;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum KnowledgeChainError {
    #[error("knowledge chain database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("hashing a knowledge entry failed: {0}")]
    Hash(#[from] serde_json::Error),
}

/// One entry in a component's knowledge chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeEntry {
    /// `kn_<ulid>` — time-sortable, mirrors `ledger::new_ev_id`'s `ev_` idiom.
    pub entry_id: String,
    pub component_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 0-based position within THIS component's chain.
    pub seq: i64,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    pub entry_hash: String,
    pub written_at: String,
}

/// The payload actually hashed for `entry_hash` — deliberately narrower than
/// `KnowledgeEntry` (excludes `entry_id`/`written_at`, which are identity and
/// bookkeeping, not content the chain is vouching for).
#[derive(Serialize)]
struct ChainedPayload<'a> {
    component_id: &'a str,
    seq: i64,
    content: &'a str,
    prev_hash: Option<&'a str>,
}

/// Where a chain's integrity broke, per `KnowledgeChain::verify_chain`.
#[derive(Debug, Clone, Serialize)]
pub struct BrokenLink {
    pub entry_id: String,
    pub seq: i64,
    /// What integrity required at this position (either the previous
    /// entry's recomputed hash, for a linkage break, or this entry's own
    /// recomputed hash, for a self-hash mismatch).
    pub expected_hash: String,
    /// What was actually found (the stored `prev_hash` or `entry_hash`,
    /// respectively).
    pub found_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChainVerification {
    pub component_id: String,
    /// How many entries, from genesis, form an intact prefix.
    pub verified_entries: u64,
    pub broken_at: Option<BrokenLink>,
}

pub struct KnowledgeChain {
    connection: Connection,
}

impl KnowledgeChain {
    pub fn open(path: &Path) -> Result<Self, KnowledgeChainError> {
        Self::initialize(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, KnowledgeChainError> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> Result<Self, KnowledgeChainError> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS knowledge_entries (
                entry_id     TEXT PRIMARY KEY,
                component_id TEXT NOT NULL,
                session_id   TEXT,
                seq          INTEGER NOT NULL,
                content      TEXT NOT NULL,
                prev_hash    TEXT,
                entry_hash   TEXT NOT NULL,
                written_at   TEXT NOT NULL
             );
             CREATE UNIQUE INDEX IF NOT EXISTS idx_knowledge_seq
                ON knowledge_entries(component_id, seq);",
        )?;
        Ok(Self { connection })
    }

    /// Appends one entry to `component_id`'s chain. The ONLY write operation
    /// over `knowledge_entries` — no UPDATE or DELETE is exposed, matching
    /// the append-only discipline of `keel-engine::ledger`.
    pub fn append(
        &self,
        component_id: &str,
        session_id: Option<&str>,
        content: &str,
    ) -> Result<KnowledgeEntry, KnowledgeChainError> {
        let head = self.head(component_id)?;
        let seq = head.as_ref().map(|h| h.seq + 1).unwrap_or(0);
        let prev_hash = head.map(|h| h.entry_hash);
        let entry_hash = ContentHash::of_canonical(&ChainedPayload {
            component_id,
            seq,
            content,
            prev_hash: prev_hash.as_deref(),
        })?
        .to_string();
        let entry = KnowledgeEntry {
            entry_id: format!("kn_{}", ulid::Ulid::new().to_string().to_lowercase()),
            component_id: component_id.to_string(),
            session_id: session_id.map(str::to_string),
            seq,
            content: content.to_string(),
            prev_hash,
            entry_hash,
            written_at: jiff::Timestamp::now().to_string(),
        };
        self.connection.execute(
            "INSERT INTO knowledge_entries
             (entry_id, component_id, session_id, seq, content, prev_hash, entry_hash, written_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                entry.entry_id,
                entry.component_id,
                entry.session_id,
                entry.seq,
                entry.content,
                entry.prev_hash,
                entry.entry_hash,
                entry.written_at,
            ],
        )?;
        Ok(entry)
    }

    /// The most recent entry of `component_id`'s chain, or `None` if it has
    /// never been written to (genesis).
    pub fn head(&self, component_id: &str) -> Result<Option<KnowledgeEntry>, KnowledgeChainError> {
        self.connection
            .query_row(
                "SELECT entry_id, component_id, session_id, seq, content, prev_hash,
                        entry_hash, written_at
                 FROM knowledge_entries WHERE component_id = ?1
                 ORDER BY seq DESC LIMIT 1",
                params![component_id],
                row_to_entry,
            )
            .optional()
            .map_err(KnowledgeChainError::Database)
    }

    /// All entries of `component_id`'s chain, genesis first.
    pub fn entries(&self, component_id: &str) -> Result<Vec<KnowledgeEntry>, KnowledgeChainError> {
        let mut stmt = self.connection.prepare(
            "SELECT entry_id, component_id, session_id, seq, content, prev_hash,
                    entry_hash, written_at
             FROM knowledge_entries WHERE component_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![component_id], row_to_entry)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(KnowledgeChainError::Database)
    }

    /// Walks `component_id`'s chain from genesis and RECOMPUTES every hash
    /// from stored content — it never trusts the `entry_hash` column at face
    /// value. Returns data, never `Err`, for "tampering found": that is a
    /// fact to report, not a failure of this call (same fail-safe posture as
    /// `keel-engine::tools`: a broken tool yields `unknown`, not a crash).
    ///
    /// Detects: (a) an entry whose stored content no longer matches its own
    /// recomputed hash, and (b) an entry whose own hash was forged to stay
    /// self-consistent, but the NEXT entry's `prev_hash` link no longer
    /// resolves to it. Does NOT detect a fully self-consistent rewrite of an
    /// entire suffix of the chain — that needs an external witness (a
    /// checkpoint anchored outside this database, e.g. in a versioned
    /// `keel.lock`), which is intentionally out of scope for this module.
    pub fn verify_chain(&self, component_id: &str) -> Result<ChainVerification, KnowledgeChainError> {
        let entries = self.entries(component_id)?;
        let mut expected_prev_hash: Option<String> = None;
        let mut verified: u64 = 0;

        for e in &entries {
            if e.prev_hash != expected_prev_hash {
                return Ok(ChainVerification {
                    component_id: component_id.to_string(),
                    verified_entries: verified,
                    broken_at: Some(BrokenLink {
                        entry_id: e.entry_id.clone(),
                        seq: e.seq,
                        expected_hash: expected_prev_hash.unwrap_or_else(|| "<genesis>".into()),
                        found_hash: e.prev_hash.clone().unwrap_or_else(|| "<genesis>".into()),
                    }),
                });
            }
            let recomputed = ContentHash::of_canonical(&ChainedPayload {
                component_id,
                seq: e.seq,
                content: &e.content,
                prev_hash: e.prev_hash.as_deref(),
            })?
            .to_string();
            if recomputed != e.entry_hash {
                return Ok(ChainVerification {
                    component_id: component_id.to_string(),
                    verified_entries: verified,
                    broken_at: Some(BrokenLink {
                        entry_id: e.entry_id.clone(),
                        seq: e.seq,
                        expected_hash: recomputed,
                        found_hash: e.entry_hash.clone(),
                    }),
                });
            }
            expected_prev_hash = Some(recomputed);
            verified += 1;
        }

        Ok(ChainVerification {
            component_id: component_id.to_string(),
            verified_entries: verified,
            broken_at: None,
        })
    }
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeEntry> {
    Ok(KnowledgeEntry {
        entry_id: row.get(0)?,
        component_id: row.get(1)?,
        session_id: row.get(2)?,
        seq: row.get(3)?,
        content: row.get(4)?,
        prev_hash: row.get(5)?,
        entry_hash: row.get(6)?,
        written_at: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_increments_seq_and_chains_prev_hash() {
        let chain = KnowledgeChain::open_in_memory().unwrap();
        let e0 = chain.append("k1", None, "first").unwrap();
        assert_eq!(e0.seq, 0);
        assert_eq!(e0.prev_hash, None);

        let e1 = chain.append("k1", Some("s1"), "second").unwrap();
        assert_eq!(e1.seq, 1);
        assert_eq!(e1.prev_hash, Some(e0.entry_hash.clone()));
        assert_ne!(e1.entry_hash, e0.entry_hash);

        // A different component starts its own chain at seq 0, independent
        // of k1's.
        let other = chain.append("k2", None, "unrelated").unwrap();
        assert_eq!(other.seq, 0);
        assert_eq!(other.prev_hash, None);

        assert_eq!(chain.head("k1").unwrap().unwrap().entry_id, e1.entry_id);
        assert_eq!(chain.entries("k1").unwrap().len(), 2);
    }

    #[test]
    fn verify_chain_is_clean_on_an_untouched_chain() {
        let chain = KnowledgeChain::open_in_memory().unwrap();
        for i in 0..5 {
            chain.append("k1", None, &format!("entry {i}")).unwrap();
        }
        let v = chain.verify_chain("k1").unwrap();
        assert_eq!(v.verified_entries, 5);
        assert!(v.broken_at.is_none());
    }

    #[test]
    fn verify_chain_detects_content_tampered_after_the_fact() {
        let chain = KnowledgeChain::open_in_memory().unwrap();
        chain.append("k1", None, "genesis").unwrap();
        let e1 = chain.append("k1", None, "second").unwrap();
        chain.append("k1", None, "third").unwrap();

        // Simulate tampering OUTSIDE the API: rewrite an old entry's content
        // without recomputing entry_hash (the naive attack).
        chain
            .connection
            .execute(
                "UPDATE knowledge_entries SET content = 'FORGED' WHERE entry_id = ?1",
                params![e1.entry_id],
            )
            .unwrap();

        let v = chain.verify_chain("k1").unwrap();
        assert_eq!(v.verified_entries, 1, "the genesis entry is still intact");
        let broken = v.broken_at.expect("tampering must be detected");
        assert_eq!(broken.entry_id, e1.entry_id);
        assert_eq!(broken.seq, 1);
    }

    #[test]
    fn verify_chain_detects_a_self_consistent_forgery_via_the_next_links_prev_hash() {
        let chain = KnowledgeChain::open_in_memory().unwrap();
        chain.append("k1", None, "genesis").unwrap();
        let e1 = chain.append("k1", None, "second").unwrap();
        chain.append("k1", None, "third").unwrap();

        // A more sophisticated attack: forge entry 1's content AND recompute
        // its own entry_hash so it is internally self-consistent. It should
        // still be caught — via entry 2's prev_hash, which still points at
        // the ORIGINAL hash of entry 1, not the forged one.
        let forged_hash = ContentHash::of_canonical(&ChainedPayload {
            component_id: "k1",
            seq: 1,
            content: "FORGED",
            prev_hash: e1.prev_hash.as_deref(),
        })
        .unwrap()
        .to_string();
        chain
            .connection
            .execute(
                "UPDATE knowledge_entries SET content = 'FORGED', entry_hash = ?1 WHERE entry_id = ?2",
                params![forged_hash, e1.entry_id],
            )
            .unwrap();

        let v = chain.verify_chain("k1").unwrap();
        assert_eq!(
            v.verified_entries, 2,
            "entry 0 and the self-consistent forged entry 1 both pass their own check"
        );
        let broken = v.broken_at.expect("the link into entry 2 must break");
        assert_eq!(broken.seq, 2);
    }
}
