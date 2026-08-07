use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use ulid::Ulid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueState {
    Pending,
    Claimed,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl QueueState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTask {
    pub id: String,
    pub session_id: String,
    pub agent_id: String,
    pub executor_id: String,
    pub state: QueueState,
}

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("scheduler database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("agent concurrency limit reached")]
    ConcurrencyLimit,
    #[error("agent task not found: {0}")]
    TaskNotFound(String),
}

pub struct AgentScheduler {
    connection: Connection,
    max_concurrent: u32,
}

impl AgentScheduler {
    pub fn in_memory(max_concurrent: u32) -> Result<Self, SchedulerError> {
        let connection = Connection::open_in_memory()?;
        let scheduler = Self {
            connection,
            max_concurrent: max_concurrent.max(1),
        };
        scheduler.initialize()?;
        Ok(scheduler)
    }

    pub fn open(path: &Path, max_concurrent: u32) -> Result<Self, SchedulerError> {
        let scheduler = Self {
            connection: Connection::open(path)?,
            max_concurrent: max_concurrent.max(1),
        };
        scheduler.initialize()?;
        Ok(scheduler)
    }

    fn initialize(&self) -> Result<(), SchedulerError> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_tasks (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                executor_id TEXT NOT NULL,
                state TEXT NOT NULL,
                lease_until INTEGER
            );",
        )?;
        Ok(())
    }

    pub fn submit(
        &self,
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        executor_id: impl Into<String>,
    ) -> Result<AgentTask, SchedulerError> {
        let task = AgentTask {
            id: Ulid::new().to_string(),
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            executor_id: executor_id.into(),
            state: QueueState::Pending,
        };
        self.connection.execute(
            "INSERT INTO agent_tasks (id, session_id, agent_id, executor_id, state)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                task.id,
                task.session_id,
                task.agent_id,
                task.executor_id,
                task.state.as_str()
            ],
        )?;
        Ok(task)
    }

    pub fn claim(&mut self) -> Result<Option<AgentTask>, SchedulerError> {
        self.prepare_claim()?;
        let tx = self.connection.transaction()?;
        let id: Option<String> = tx
            .query_row(
                "SELECT id FROM agent_tasks WHERE state = 'pending' ORDER BY rowid LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            return Ok(None);
        };
        let lease_until = now_seconds() + 60;
        tx.execute(
            "UPDATE agent_tasks SET state = 'claimed', lease_until = ?2
             WHERE id = ?1 AND state = 'pending'",
            params![id, lease_until],
        )?;
        let task = read_task(&tx, &id)?.ok_or_else(|| SchedulerError::TaskNotFound(id.clone()))?;
        tx.commit()?;
        Ok(Some(task))
    }

    pub fn claim_task(&mut self, task_id: &str) -> Result<Option<AgentTask>, SchedulerError> {
        self.prepare_claim()?;
        let tx = self.connection.transaction()?;
        let changed = tx.execute(
            "UPDATE agent_tasks SET state = 'claimed', lease_until = ?2
             WHERE id = ?1 AND state = 'pending'",
            params![task_id, now_seconds() + 60],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        let task = read_task(&tx, task_id)?
            .ok_or_else(|| SchedulerError::TaskNotFound(task_id.to_string()))?;
        tx.commit()?;
        Ok(Some(task))
    }

    pub fn start(&self, task_id: &str) -> Result<(), SchedulerError> {
        let changed = self.connection.execute(
            "UPDATE agent_tasks SET state = 'running', lease_until = ?2
             WHERE id = ?1 AND state = 'claimed'",
            params![task_id, now_seconds() + 60],
        )?;
        if changed == 0 {
            return Err(SchedulerError::TaskNotFound(task_id.to_string()));
        }
        Ok(())
    }

    pub fn renew(&self, task_id: &str) -> Result<(), SchedulerError> {
        let changed = self.connection.execute(
            "UPDATE agent_tasks SET lease_until = ?2
             WHERE id = ?1 AND state IN ('claimed', 'running')",
            params![task_id, now_seconds() + 60],
        )?;
        if changed == 0 {
            return Err(SchedulerError::TaskNotFound(task_id.to_string()));
        }
        Ok(())
    }

    pub fn complete(&self, task_id: &str) -> Result<(), SchedulerError> {
        self.update_state(task_id, QueueState::Completed)
    }

    pub fn fail(&self, task_id: &str) -> Result<(), SchedulerError> {
        self.update_state(task_id, QueueState::Failed)
    }

    pub fn cancel(&self, task_id: &str) -> Result<(), SchedulerError> {
        self.update_state(task_id, QueueState::Cancelled)
    }

    pub fn state(&self, task_id: &str) -> Result<QueueState, SchedulerError> {
        read_task(&self.connection, task_id)?
            .map(|task| task.state)
            .ok_or_else(|| SchedulerError::TaskNotFound(task_id.to_string()))
    }

    fn update_state(&self, task_id: &str, state: QueueState) -> Result<(), SchedulerError> {
        let changed = self.connection.execute(
            "UPDATE agent_tasks SET state = ?2, lease_until = NULL WHERE id = ?1",
            params![task_id, state.as_str()],
        )?;
        if changed == 0 {
            return Err(SchedulerError::TaskNotFound(task_id.to_string()));
        }
        Ok(())
    }

    fn prepare_claim(&self) -> Result<(), SchedulerError> {
        self.connection.execute(
            "UPDATE agent_tasks SET state = 'pending', lease_until = NULL
             WHERE state IN ('claimed', 'running') AND lease_until < ?1",
            params![now_seconds()],
        )?;
        let running: u32 = self.connection.query_row(
            "SELECT COUNT(*) FROM agent_tasks WHERE state IN ('claimed', 'running')",
            [],
            |row| row.get(0),
        )?;
        if running >= self.max_concurrent {
            return Err(SchedulerError::ConcurrencyLimit);
        }
        Ok(())
    }
}

fn read_task(connection: &Connection, id: &str) -> Result<Option<AgentTask>, rusqlite::Error> {
    connection
        .query_row(
            "SELECT id, session_id, agent_id, executor_id, state
             FROM agent_tasks WHERE id = ?1",
            params![id],
            |row| {
                let state: String = row.get(4)?;
                Ok(AgentTask {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    agent_id: row.get(2)?,
                    executor_id: row.get(3)?,
                    state: match state.as_str() {
                        "pending" => QueueState::Pending,
                        "claimed" => QueueState::Claimed,
                        "running" => QueueState::Running,
                        "completed" => QueueState::Completed,
                        "failed" => QueueState::Failed,
                        "cancelled" => QueueState::Cancelled,
                        _ => QueueState::Failed,
                    },
                })
            },
        )
        .optional()
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after epoch")
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::{AgentScheduler, QueueState, SchedulerError};

    #[test]
    fn scheduler_enforces_concurrency_and_tracks_lifecycle() {
        let mut scheduler = AgentScheduler::in_memory(1).unwrap();
        let first = scheduler.submit("s", "reviewer", "codex").unwrap();
        let second = scheduler.submit("s", "tester", "claude").unwrap();
        let claimed = scheduler.claim().unwrap().unwrap();
        assert_eq!(claimed.id, first.id);
        scheduler.start(&first.id).unwrap();
        assert!(matches!(
            scheduler.claim(),
            Err(SchedulerError::ConcurrencyLimit)
        ));
        scheduler.complete(&first.id).unwrap();
        assert_eq!(scheduler.state(&first.id).unwrap(), QueueState::Completed);
        assert_eq!(scheduler.claim().unwrap().unwrap().id, second.id);
    }
}
