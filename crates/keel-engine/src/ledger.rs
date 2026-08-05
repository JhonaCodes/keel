// SPDX-License-Identifier: Apache-2.0
//! Evidence Ledger — THE FIRST PRODUCT of the system (spec §6.4, ADR-021).
//!
//! Enforcement is the second product; this is the first thing that gets
//! built and the first thing that delivers value: constraint telemetry
//! answers which rules are alive, dead, or mis-specified — something no
//! instruction-file can answer today.
//!
//! The ledger records HOW something was known, not just what was known: each
//! entry carries its origin class (deterministic/semantic/attestation/human)
//! and the classes are never mixed.
//!
//! APPEND-ONLY (invariant 16 via API surface): this module exposes no UPDATE
//! or DELETE over the evidence. Human prune decisions are recorded as NEW
//! entries in their own table, with class `human` (§7.7).
//!
//! BOUNDARY RULES: does not import `keel_dsl` (⇏ dsl) and no ledger module
//! calls the runtime (⇏ runtime: the ledger is a sink — the runtime writes
//! to it, never the other way around).

use keel_core::event::EventKind;
use keel_core::{Decision, OriginClass, Verdict};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One evidence entry (spec §6.4: rule and version, verdict, origin class,
/// cost, and resulting decision).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// `ev_<ulid>` — time-sortable, citable in the transcript (§10.4).
    pub id: String,
    /// RFC 3339.
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub snapshot_hash: String,
    pub rule_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_version: Option<String>,
    pub event_kind: EventKind,
    pub verdict: Verdict,
    pub origin: OriginClass,
    /// What the rule ASKED for. In passive mode it is preserved intact: it is
    /// the datum Phase 0c will measure ("what enforcement would have done").
    pub declared_decision: Decision,
    /// What was actually applied. Phase 0: never above `review`.
    pub effective_decision: Decision,
    pub latency_ms: u64,
    /// 0 for every deterministic tool — the economics of §4.4 made into data.
    pub tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Messages from findings / failed precondition / recorded invoke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Aggregated per-rule telemetry (spec §6.4, the operational-questions table).
#[derive(Debug, Clone, Serialize)]
pub struct RuleStats {
    pub rule_id: String,
    pub evaluations: u64,
    pub invalid: u64,
    pub unknown: u64,
    pub valid: u64,
    pub first_ts: String,
    pub last_ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_invalid_ts: Option<String>,
    pub avg_latency_ms: f64,
    pub total_tokens: u64,
}

/// Repetition of the same finding (rule+location) within a session —
/// the oscillation signal of §6.5.
#[derive(Debug, Clone, Serialize)]
pub struct Oscillation {
    pub rule_id: String,
    pub session_id: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub count: u64,
}

/// Recorded human decision (§7.7: deletion stops being risky because it
/// stops being blind — and it stays audited with class `human`).
#[derive(Debug, Clone, Serialize)]
pub struct HumanDecision {
    pub id: String,
    pub ts: String,
    pub rule_id: String,
    /// keep | adjust | prune
    pub decision: String,
    pub decided_by: String,
    pub reason: Option<String>,
}

pub struct Ledger {
    conn: Connection,
}

impl Ledger {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS evidence (
              id                 TEXT PRIMARY KEY,
              ts                 TEXT NOT NULL,
              session_id         TEXT,
              snapshot_hash      TEXT NOT NULL,
              rule_id            TEXT NOT NULL,
              rule_version       TEXT,
              event_kind         TEXT NOT NULL,
              verdict            TEXT NOT NULL,
              origin             TEXT NOT NULL,
              declared_decision  TEXT NOT NULL,
              effective_decision TEXT NOT NULL,
              latency_ms         INTEGER NOT NULL,
              tokens             INTEGER NOT NULL,
              file               TEXT,
              line               INTEGER,
              detail             TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_evidence_rule ON evidence(rule_id);
            CREATE INDEX IF NOT EXISTS idx_evidence_session ON evidence(session_id);

            -- Human lifecycle decisions (§7.7). Origin both implicit and
            -- explicit: class `human`, always.
            CREATE TABLE IF NOT EXISTS human_decisions (
              id         TEXT PRIMARY KEY,
              ts         TEXT NOT NULL,
              rule_id    TEXT NOT NULL,
              origin     TEXT NOT NULL DEFAULT 'human',
              decision   TEXT NOT NULL,
              decided_by TEXT NOT NULL,
              reason     TEXT
            );
            "#,
        )?;
        Ok(Ledger { conn })
    }

    /// Appends evidence. The ONLY write operation over `evidence`.
    pub fn append(&self, e: &LedgerEntry) -> rusqlite::Result<()> {
        self.conn.execute(
            r#"INSERT INTO evidence
               (id, ts, session_id, snapshot_hash, rule_id, rule_version,
                event_kind, verdict, origin, declared_decision,
                effective_decision, latency_ms, tokens, file, line, detail)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)"#,
            params![
                e.id,
                e.ts,
                e.session_id,
                e.snapshot_hash,
                e.rule_id,
                e.rule_version,
                json_str(&e.event_kind),
                json_str(&e.verdict),
                json_str(&e.origin),
                json_str(&e.declared_decision),
                json_str(&e.effective_decision),
                e.latency_ms,
                e.tokens,
                e.file,
                e.line,
                e.detail,
            ],
        )?;
        Ok(())
    }

    /// Resolves an entry by id — the basis of `keel explain`.
    pub fn get(&self, ev_id: &str) -> rusqlite::Result<Option<LedgerEntry>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, ts, session_id, snapshot_hash, rule_id, rule_version,
                      event_kind, verdict, origin, declared_decision,
                      effective_decision, latency_ms, tokens, file, line, detail
               FROM evidence WHERE id = ?1"#,
        )?;
        let mut rows = stmt.query(params![ev_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_entry(row)?)),
            None => Ok(None),
        }
    }

    /// Per-rule telemetry (§6.4): the operational-questions table.
    pub fn rule_stats(&self) -> rusqlite::Result<Vec<RuleStats>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT rule_id,
                      COUNT(*)                                            AS evaluations,
                      SUM(verdict = '"invalid"')                          AS invalid,
                      SUM(verdict = '"unknown"')                          AS unknown,
                      SUM(verdict = '"valid"')                            AS valid,
                      MIN(ts)                                             AS first_ts,
                      MAX(ts)                                             AS last_ts,
                      MAX(CASE WHEN verdict = '"invalid"' THEN ts END)    AS last_invalid_ts,
                      AVG(latency_ms)                                     AS avg_latency,
                      SUM(tokens)                                         AS total_tokens
               FROM evidence GROUP BY rule_id ORDER BY rule_id"#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RuleStats {
                rule_id: row.get(0)?,
                evaluations: row.get(1)?,
                invalid: row.get::<_, Option<u64>>(2)?.unwrap_or(0),
                unknown: row.get::<_, Option<u64>>(3)?.unwrap_or(0),
                valid: row.get::<_, Option<u64>>(4)?.unwrap_or(0),
                first_ts: row.get(5)?,
                last_ts: row.get(6)?,
                last_invalid_ts: row.get(7)?,
                avg_latency_ms: row.get::<_, Option<f64>>(8)?.unwrap_or(0.0),
                total_tokens: row.get::<_, Option<u64>>(9)?.unwrap_or(0),
            })
        })?;
        rows.collect()
    }

    /// Repeated findings (rule+location per session) — §6.5 oscillation.
    pub fn oscillations(&self, threshold: u64) -> rusqlite::Result<Vec<Oscillation>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT rule_id, session_id, file, line, COUNT(*) AS n
               FROM evidence
               WHERE verdict = '"invalid"'
               GROUP BY rule_id, session_id, file, line
               HAVING COUNT(*) >= ?1
               ORDER BY n DESC"#,
        )?;
        let rows = stmt.query_map(params![threshold], |row| {
            Ok(Oscillation {
                rule_id: row.get(0)?,
                session_id: row.get(1)?,
                file: row.get(2)?,
                line: row.get(3)?,
                count: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Records a human lifecycle decision (§7.7). Deleting a rule is NEVER
    /// executed by the system: it proposes with data; a human decides; the
    /// trail stays here with class `human`.
    pub fn record_human_decision(
        &self,
        rule_id: &str,
        decision: &str,
        decided_by: &str,
        reason: Option<&str>,
        id: &str,
        ts: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            r#"INSERT INTO human_decisions (id, ts, rule_id, origin, decision, decided_by, reason)
               VALUES (?1, ?2, ?3, 'human', ?4, ?5, ?6)"#,
            params![id, ts, rule_id, decision, decided_by, reason],
        )?;
        Ok(())
    }

    pub fn human_decisions(&self, rule_id: Option<&str>) -> rusqlite::Result<Vec<HumanDecision>> {
        fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HumanDecision> {
            Ok(HumanDecision {
                id: row.get(0)?,
                ts: row.get(1)?,
                rule_id: row.get(2)?,
                decision: row.get(3)?,
                decided_by: row.get(4)?,
                reason: row.get(5)?,
            })
        }
        match rule_id {
            Some(r) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, ts, rule_id, decision, decided_by, reason
                     FROM human_decisions WHERE rule_id = ?1 ORDER BY ts",
                )?;
                let rows = stmt.query_map(params![r], map_row)?;
                rows.collect()
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, ts, rule_id, decision, decided_by, reason
                     FROM human_decisions ORDER BY ts",
                )?;
                let rows = stmt.query_map([], map_row)?;
                rows.collect()
            }
        }
    }

    /// Unresolved blockers of a session (§6.2 acceptance, minimal form):
    /// invalid findings whose declared decision is blocking and for which no
    /// LATER `valid` evaluation of the same rule+file exists in the session.
    /// Feeds the completion gate (§12.3: completion requires runtime
    /// authorization — you cannot declare "done" over live blockers).
    pub fn unresolved_blockers(&self, session_id: &str) -> rusqlite::Result<Vec<LedgerEntry>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, ts, session_id, snapshot_hash, rule_id, rule_version,
                      event_kind, verdict, origin, declared_decision,
                      effective_decision, latency_ms, tokens, file, line, detail
               FROM evidence e
               WHERE e.session_id = ?1
                 AND e.verdict = '"invalid"'
                 AND e.declared_decision IN ('"block"', '"deny-pending-approval"')
                 AND NOT EXISTS (
                   SELECT 1 FROM evidence v
                   WHERE v.session_id = e.session_id
                     AND v.rule_id = e.rule_id
                     AND (v.file = e.file OR (v.file IS NULL AND e.file IS NULL))
                     AND v.verdict = '"valid"'
                     AND v.ts > e.ts
                 )
               ORDER BY e.ts"#,
        )?;
        let rows = stmt.query_map(rusqlite::params![session_id], |row| row_to_entry(row))?;
        rows.collect()
    }

    pub fn count(&self) -> rusqlite::Result<u64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM evidence", [], |r| r.get(0))
    }
}

/// Enums travel to SQLite in their canonical serde form (`"invalid"`, with
/// JSON quotes): a single textual representation across the whole system —
/// the same one that appears in the snapshot, the ledger, and CLI output.
fn json_str<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("vocabulary enums always serialize")
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<LedgerEntry> {
    fn parse<T: for<'de> Deserialize<'de>>(idx: usize, row: &rusqlite::Row<'_>) -> rusqlite::Result<T> {
        let raw: String = row.get(idx)?;
        serde_json::from_str(&raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                idx,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })
    }
    Ok(LedgerEntry {
        id: row.get(0)?,
        ts: row.get(1)?,
        session_id: row.get(2)?,
        snapshot_hash: row.get(3)?,
        rule_id: row.get(4)?,
        rule_version: row.get(5)?,
        event_kind: parse(6, row)?,
        verdict: parse(7, row)?,
        origin: parse(8, row)?,
        declared_decision: parse(9, row)?,
        effective_decision: parse(10, row)?,
        latency_ms: row.get(11)?,
        tokens: row.get(12)?,
        file: row.get(13)?,
        line: row.get(14)?,
        detail: row.get(15)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, rule: &str, verdict: Verdict, ts: &str) -> LedgerEntry {
        LedgerEntry {
            id: id.into(),
            ts: ts.into(),
            session_id: Some("s1".into()),
            snapshot_hash: "sha256:x".into(),
            rule_id: rule.into(),
            rule_version: Some("1.0.0".into()),
            event_kind: EventKind::FileEdited,
            verdict,
            origin: OriginClass::Deterministic,
            declared_decision: Decision::Block,
            effective_decision: Decision::Review,
            latency_ms: 5,
            tokens: 0,
            file: Some("lib/a.dart".into()),
            line: Some(10),
            detail: None,
        }
    }

    #[test]
    fn append_and_get_roundtrip() {
        let ledger = Ledger::open_in_memory().unwrap();
        let e = entry("ev_1", "r1", Verdict::Invalid, "2026-01-01T00:00:00Z");
        ledger.append(&e).unwrap();
        let back = ledger.get("ev_1").unwrap().unwrap();
        assert_eq!(back.rule_id, "r1");
        assert_eq!(back.verdict, Verdict::Invalid);
        assert_eq!(back.declared_decision, Decision::Block);
        assert_eq!(back.effective_decision, Decision::Review);
        assert_eq!(back.origin, OriginClass::Deterministic);
    }

    /// The telemetry of §6.4 is THE PRODUCT — it gets tested as a product.
    #[test]
    fn rule_stats_answer_the_operational_questions() {
        let ledger = Ledger::open_in_memory().unwrap();
        // r-hot always fires; r-dead never; r-fuzzy comes back unknown.
        for i in 0..5 {
            ledger
                .append(&entry(&format!("ev_h{i}"), "r-hot", Verdict::Invalid, "2026-01-02T00:00:00Z"))
                .unwrap();
        }
        for i in 0..10 {
            ledger
                .append(&entry(&format!("ev_d{i}"), "r-dead", Verdict::Valid, "2026-01-03T00:00:00Z"))
                .unwrap();
        }
        for i in 0..4 {
            ledger
                .append(&entry(&format!("ev_f{i}"), "r-fuzzy", Verdict::Unknown, "2026-01-04T00:00:00Z"))
                .unwrap();
        }

        let stats = ledger.rule_stats().unwrap();
        let by_id = |id: &str| stats.iter().find(|s| s.rule_id == id).unwrap();

        let hot = by_id("r-hot");
        assert_eq!((hot.evaluations, hot.invalid), (5, 5)); // the pattern is wrong, not the devs
        let dead = by_id("r-dead");
        assert_eq!((dead.evaluations, dead.invalid), (10, 0)); // prune candidate
        let fuzzy = by_id("r-fuzzy");
        assert_eq!((fuzzy.evaluations, fuzzy.unknown), (4, 4)); // mis-specified
    }

    /// §6.5 oscillation: same rule+location repeated within a session.
    #[test]
    fn oscillation_detects_repeated_findings_at_same_location() {
        let ledger = Ledger::open_in_memory().unwrap();
        for i in 0..3 {
            ledger
                .append(&entry(&format!("ev_{i}"), "r1", Verdict::Invalid, "2026-01-01T00:00:00Z"))
                .unwrap();
        }
        ledger
            .append(&entry("ev_other", "r2", Verdict::Invalid, "2026-01-01T00:00:00Z"))
            .unwrap();

        let osc = ledger.oscillations(3).unwrap();
        assert_eq!(osc.len(), 1);
        assert_eq!(osc[0].rule_id, "r1");
        assert_eq!(osc[0].count, 3);
    }

    #[test]
    fn human_decisions_are_recorded_with_human_class() {
        let ledger = Ledger::open_in_memory().unwrap();
        ledger
            .record_human_decision("r-dead", "prune", "jhonatan", Some("0 fires in P6M"), "hd_1", "2026-01-05T00:00:00Z")
            .unwrap();
        let ds = ledger.human_decisions(Some("r-dead")).unwrap();
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].decision, "prune");
        assert_eq!(ds[0].decided_by, "jhonatan");
    }
}