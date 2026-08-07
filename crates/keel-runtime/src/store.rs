use crate::{
    ArtifactKind, ArtifactReceipt, ComponentReadReceipt, Phase, PhaseTransitionReceipt,
    SkillReadReceipt,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("runtime store error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(
        "session `{session_id}` belongs to snapshot `{stored}`, not requested snapshot `{requested}`"
    )]
    SnapshotMismatch {
        session_id: String,
        stored: String,
        requested: String,
    },
    #[error("session `{session_id}` has different persisted execution metadata")]
    MetadataMismatch { session_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadata {
    pub task: String,
    pub executor_id: String,
}

/// Durable, append-only evidence owned by the governed runtime.
pub struct RuntimeStore {
    connection: Connection,
}

impl RuntimeStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::initialize(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> Result<Self, StoreError> {
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS runtime_sessions (
                session_id TEXT PRIMARY KEY,
                snapshot_hash TEXT NOT NULL,
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS component_receipts (
                receipt_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                component_kind TEXT NOT NULL,
                component_id TEXT NOT NULL,
                version TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                phase TEXT NOT NULL,
                reason TEXT,
                required INTEGER NOT NULL,
                read_at TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES runtime_sessions(session_id)
             );
             CREATE TABLE IF NOT EXISTS session_metadata (
                session_id TEXT PRIMARY KEY,
                task TEXT NOT NULL,
                executor_id TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES runtime_sessions(session_id)
             );
             CREATE INDEX IF NOT EXISTS idx_component_receipts_session
                ON component_receipts(session_id, component_kind, component_id);
             CREATE TABLE IF NOT EXISTS runtime_artifacts (
                artifact_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                schema_valid INTEGER NOT NULL,
                recorded_at TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES runtime_sessions(session_id)
             );
             CREATE TABLE IF NOT EXISTS phase_transitions (
                transition_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                from_phase TEXT NOT NULL,
                to_phase TEXT NOT NULL,
                guard_artifact_id TEXT NOT NULL,
                advanced_at TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES runtime_sessions(session_id),
                FOREIGN KEY(guard_artifact_id) REFERENCES runtime_artifacts(artifact_id)
             );
             CREATE INDEX IF NOT EXISTS idx_runtime_artifacts_session
                ON runtime_artifacts(session_id, kind);
             CREATE INDEX IF NOT EXISTS idx_phase_transitions_session
                ON phase_transitions(session_id);",
        )?;
        Ok(Self { connection })
    }

    pub fn ensure_session(&self, session_id: &str, snapshot_hash: &str) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT OR IGNORE INTO runtime_sessions (session_id, snapshot_hash, created_at)
             VALUES (?1, ?2, ?3)",
            params![
                session_id,
                snapshot_hash,
                jiff::Timestamp::now().to_string()
            ],
        )?;
        let stored: String = self.connection.query_row(
            "SELECT snapshot_hash FROM runtime_sessions WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        if stored != snapshot_hash {
            return Err(StoreError::SnapshotMismatch {
                session_id: session_id.to_string(),
                stored,
                requested: snapshot_hash.to_string(),
            });
        }
        Ok(())
    }

    pub fn append_skill_receipt(&self, receipt: &SkillReadReceipt) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO component_receipts
             (receipt_id, session_id, component_kind, component_id, version,
              content_hash, phase, reason, required, read_at)
             VALUES (?1, ?2, 'skill', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                receipt.receipt_id,
                receipt.session_id,
                receipt.skill_id,
                receipt.version,
                receipt.content_hash,
                receipt.phase,
                receipt.reason,
                receipt.required,
                receipt.read_at,
            ],
        )?;
        Ok(())
    }

    pub fn ensure_session_metadata(
        &self,
        session_id: &str,
        task: &str,
        executor_id: &str,
    ) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT OR IGNORE INTO session_metadata (session_id, task, executor_id)
             VALUES (?1, ?2, ?3)",
            params![session_id, task, executor_id],
        )?;
        let stored =
            self.session_metadata(session_id)?
                .ok_or_else(|| StoreError::MetadataMismatch {
                    session_id: session_id.to_string(),
                })?;
        if stored.task != task || stored.executor_id != executor_id {
            return Err(StoreError::MetadataMismatch {
                session_id: session_id.to_string(),
            });
        }
        Ok(())
    }

    pub fn session_metadata(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionMetadata>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT task, executor_id FROM session_metadata WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok(SessionMetadata {
                        task: row.get(0)?,
                        executor_id: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn append_component_receipt(
        &self,
        receipt: &ComponentReadReceipt,
    ) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO component_receipts
             (receipt_id, session_id, component_kind, component_id, version,
              content_hash, phase, reason, required, read_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                receipt.receipt_id,
                receipt.session_id,
                receipt.component_kind,
                receipt.component_id,
                receipt.version,
                receipt.content_hash,
                receipt.phase,
                receipt.reason,
                receipt.required,
                receipt.read_at,
            ],
        )?;
        Ok(())
    }

    pub fn consumed_component_keys(
        &self,
        session_id: &str,
    ) -> Result<Vec<(String, String)>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT component_kind, component_id FROM component_receipts
             WHERE session_id = ?1 ORDER BY component_kind, component_id",
        )?;
        let rows =
            statement.query_map(params![session_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn consumed_skill_ids(&self, session_id: &str) -> Result<Vec<String>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT component_id FROM component_receipts
             WHERE session_id = ?1 AND component_kind = 'skill'
             ORDER BY component_id",
        )?;
        let rows = statement.query_map(params![session_id], |row| row.get(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn receipt_count(&self, session_id: &str) -> Result<u64, StoreError> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM component_receipts WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?)
    }

    pub fn snapshot_for_session(&self, session_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT snapshot_hash FROM runtime_sessions WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn append_artifact(&self, receipt: &ArtifactReceipt) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO runtime_artifacts
             (artifact_id, session_id, kind, content_hash, schema_valid, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                receipt.artifact_id,
                receipt.session_id,
                receipt.kind.as_str(),
                receipt.content_hash,
                receipt.schema_valid,
                receipt.recorded_at,
            ],
        )?;
        Ok(())
    }

    pub fn artifacts_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<ArtifactReceipt>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT artifact_id, session_id, kind, content_hash, schema_valid, recorded_at
             FROM runtime_artifacts WHERE session_id = ?1 ORDER BY rowid",
        )?;
        let rows = statement.query_map(params![session_id], |row| {
            let raw_kind: String = row.get(2)?;
            let kind = ArtifactKind::parse(&raw_kind).ok_or_else(|| conversion_error(2))?;
            Ok(ArtifactReceipt {
                artifact_id: row.get(0)?,
                session_id: row.get(1)?,
                kind,
                content_hash: row.get(3)?,
                schema_valid: row.get(4)?,
                recorded_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn append_transition(&self, receipt: &PhaseTransitionReceipt) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO phase_transitions
             (transition_id, session_id, from_phase, to_phase, guard_artifact_id, advanced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                receipt.transition_id,
                receipt.session_id,
                receipt.from.as_str(),
                receipt.to.as_str(),
                receipt.guard_artifact_id,
                receipt.advanced_at,
            ],
        )?;
        Ok(())
    }

    pub fn transitions_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<PhaseTransitionReceipt>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT transition_id, session_id, from_phase, to_phase,
                    guard_artifact_id, advanced_at
             FROM phase_transitions WHERE session_id = ?1 ORDER BY rowid",
        )?;
        let rows = statement.query_map(params![session_id], |row| {
            let raw_from: String = row.get(2)?;
            let raw_to: String = row.get(3)?;
            let from = Phase::parse(&raw_from).ok_or_else(|| conversion_error(2))?;
            let to = Phase::parse(&raw_to).ok_or_else(|| conversion_error(3))?;
            Ok(PhaseTransitionReceipt {
                transition_id: row.get(0)?,
                session_id: row.get(1)?,
                from,
                to,
                guard_artifact_id: row.get(4)?,
                advanced_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

fn conversion_error(index: usize) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(StoreValueError),
    )
}

#[derive(Debug)]
struct StoreValueError;

impl std::fmt::Display for StoreValueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("invalid persisted runtime value")
    }
}

impl std::error::Error for StoreValueError {}
