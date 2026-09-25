//! Durable task storage and idempotent Kubernetes Job reconciliation.

use ayni_orchestrator_domain::{
    AttemptId, AttemptOutcome, AttemptState, GitRevision, IdempotencyKey, Sha256Digest,
    TaskDesiredState, TaskId,
};
use k8s_openapi::api::batch::v1::Job;
use kube::{
    Api, Client,
    api::{DeleteParams, PostParams},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    future::Future,
    path::Path,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tracing::{info, warn};

/// The durable `SQLite` handle shared by the API and reconciler.
pub type StoreHandle = Arc<Mutex<Store>>;

/// Immutable inputs supplied when a task is submitted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskInput {
    /// Source repository to check out.
    pub repository_url: String,
    /// Exact source revision.
    pub revision: String,
    /// Expected digest of the checked-out Ayni lock.
    pub lock_sha256: String,
    /// Requested Ayni executable version.
    pub ayni_version: String,
}

impl TaskInput {
    /// Validates immutable values before storing them.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when an identity is malformed or a text field is empty.
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.repository_url.is_empty() || self.repository_url.len() > 2048 {
            return Err(StoreError::InvalidInput(
                "repository_url must be 1 to 2048 characters",
            ));
        }
        if self.ayni_version.is_empty() || self.ayni_version.len() > 64 {
            return Err(StoreError::InvalidInput(
                "ayni_version must be 1 to 64 characters",
            ));
        }
        GitRevision::parse(&self.revision).map_err(StoreError::InvalidIdentity)?;
        Sha256Digest::parse(&self.lock_sha256).map_err(StoreError::InvalidIdentity)?;
        Ok(())
    }
}

/// The task state returned by the control-plane API.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskView {
    /// Durable task identity.
    pub id: TaskId,
    /// Requested task outcome.
    pub desired_state: TaskDesiredState,
    /// Immutable inputs bound to every attempt.
    pub input: TaskInput,
    /// Maximum number of attempts that may be created for this task.
    pub max_attempts: u32,
    /// Attempts in creation order.
    pub attempts: Vec<AttemptView>,
}

/// One attempt visible to an API caller.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AttemptView {
    /// Durable attempt identity.
    pub id: AttemptId,
    /// One-based task-local attempt ordinal.
    pub ordinal: u32,
    /// Durable lifecycle state.
    pub state: AttemptState,
    /// Deterministic Kubernetes Job identity, once scheduled.
    pub job_name: Option<String>,
    /// Immutable worker image used after the attempt is scheduled.
    pub worker_image: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ReconciliationTarget {
    pub(crate) task_id: TaskId,
    pub(crate) desired_state: TaskDesiredState,
    pub(crate) attempt: AttemptView,
    pub(crate) input: TaskInput,
}

/// SQLite-backed durable task and attempt state.
pub struct Store {
    pub(crate) connection: Connection,
    max_attempts: u32,
}

impl Store {
    /// Opens and migrates a persistent store.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if the directory cannot be created or `SQLite` cannot be opened.
    pub fn open(path: impl AsRef<Path>, max_attempts: u32) -> Result<Self, StoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(StoreError::CreateStoreDirectory)?;
        }
        let connection = Connection::open(path).map_err(StoreError::Database)?;
        Self::from_connection(connection, max_attempts)
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Result<Self, StoreError> {
        Self::in_memory_with_max_attempts(3)
    }

    #[cfg(test)]
    pub(crate) fn in_memory_with_max_attempts(max_attempts: u32) -> Result<Self, StoreError> {
        Self::from_connection(
            Connection::open_in_memory().map_err(StoreError::Database)?,
            max_attempts,
        )
    }

    pub(crate) fn from_connection(
        connection: Connection,
        max_attempts: u32,
    ) -> Result<Self, StoreError> {
        if max_attempts == 0 {
            return Err(StoreError::InvalidMaxAttempts(max_attempts));
        }
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS tasks (
                    id TEXT PRIMARY KEY NOT NULL,
                    repository_url TEXT NOT NULL,
                    revision TEXT NOT NULL,
                    lock_sha256 TEXT NOT NULL,
                    ayni_version TEXT NOT NULL,
                    desired_state TEXT NOT NULL,
                    max_attempts INTEGER NOT NULL DEFAULT 3 CHECK(max_attempts > 0),
                    created_at INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS attempts (
                    id TEXT PRIMARY KEY NOT NULL,
                    task_id TEXT NOT NULL REFERENCES tasks(id),
                    ordinal INTEGER NOT NULL,
                    state TEXT NOT NULL,
                    job_name TEXT UNIQUE,
                    worker_image TEXT,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    UNIQUE(task_id, ordinal)
                 );
                 CREATE TABLE IF NOT EXISTS external_actions (
                    action_key TEXT PRIMARY KEY NOT NULL,
                    action_kind TEXT NOT NULL,
                    task_id TEXT NOT NULL REFERENCES tasks(id),
                    attempt_id TEXT REFERENCES attempts(id),
                    created_at INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS attempt_reports (
                    attempt_id TEXT NOT NULL REFERENCES attempts(id),
                    report_key TEXT NOT NULL,
                    outcome TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    PRIMARY KEY(attempt_id, report_key)
                 );
                 CREATE INDEX IF NOT EXISTS attempts_reconciliation_idx
                    ON attempts(state, updated_at);",
            )
            .map_err(StoreError::Database)?;
        migrate_task_max_attempts(&connection)?;
        let store = Self {
            connection,
            max_attempts,
        };
        store.migrate_development()?;
        store.migrate_preparations()?;
        Ok(store)
    }

    /// Submits a task exactly once for an idempotency key.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] for invalid input or database failures.
    pub fn submit(
        &mut self,
        action_key: &IdempotencyKey,
        input: &TaskInput,
    ) -> Result<TaskView, StoreError> {
        input.validate()?;
        if let Some(existing) = self.action_target(action_key.as_str())? {
            return self.task(existing.0);
        }
        let now = unix_seconds()?;
        let task_id = TaskId::generate();
        let attempt_id = AttemptId::generate();
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        insert_task(&transaction, task_id, input, self.max_attempts, now)?;
        insert_attempt(
            &transaction,
            attempt_id,
            task_id,
            1,
            AttemptState::Queued,
            now,
        )?;
        insert_action(
            &transaction,
            action_key.as_str(),
            "submit",
            task_id,
            Some(attempt_id),
            now,
        )?;
        transaction.commit().map_err(StoreError::Database)?;
        self.task(task_id)
    }

    /// Returns a task and its complete attempt history.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::TaskNotFound`] when `id` is unknown.
    pub fn task(&self, id: TaskId) -> Result<TaskView, StoreError> {
        let task_text = id.to_string();
        let row = self
            .connection
            .query_row(
                "SELECT repository_url, revision, lock_sha256, ayni_version, desired_state, max_attempts
                 FROM tasks WHERE id = ?1",
                [&task_text],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, u32>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(StoreError::Database)?
            .ok_or(StoreError::TaskNotFound(id))?;
        let input = TaskInput {
            repository_url: row.0,
            revision: row.1,
            lock_sha256: row.2,
            ayni_version: row.3,
        };
        let desired_state = parse_desired_state(&row.4)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, ordinal, state, job_name, worker_image FROM attempts
                 WHERE task_id = ?1 ORDER BY ordinal ASC",
            )
            .map_err(StoreError::Database)?;
        let attempts = statement
            .query_map([task_text], |row| {
                let id: String = row.get(0)?;
                let ordinal: u32 = row.get(1)?;
                let state: String = row.get(2)?;
                Ok((
                    id,
                    ordinal,
                    state,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(StoreError::Database)?
            .map(|row| {
                let (attempt_id, ordinal, state, job_name, worker_image) =
                    row.map_err(StoreError::Database)?;
                Ok(AttemptView {
                    id: AttemptId::parse(&attempt_id).map_err(StoreError::InvalidLifecycleData)?,
                    ordinal,
                    state: parse_attempt_state(&state)?,
                    job_name,
                    worker_image,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        Ok(TaskView {
            id,
            desired_state,
            input,
            max_attempts: row.5,
            attempts,
        })
    }

    /// Creates one retry attempt, deduplicated by a durable action key.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the task cannot be retried or the action is invalid.
    pub fn retry(
        &mut self,
        task_id: TaskId,
        action_key: &IdempotencyKey,
    ) -> Result<TaskView, StoreError> {
        if let Some((existing_task, _)) = self.action_target(action_key.as_str())? {
            if existing_task != task_id {
                return Err(StoreError::ActionKeyReused(action_key.as_str().to_owned()));
            }
            return self.task(task_id);
        }
        let task = self.task(task_id)?;
        let ordinal = retry_ordinal(&task)?;
        let attempt_id = AttemptId::generate();
        let now = unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        insert_attempt(
            &transaction,
            attempt_id,
            task_id,
            ordinal,
            AttemptState::Queued,
            now,
        )?;
        insert_action(
            &transaction,
            action_key.as_str(),
            "retry",
            task_id,
            Some(attempt_id),
            now,
        )?;
        transaction.commit().map_err(StoreError::Database)?;
        self.task(task_id)
    }

    /// Requests cancellation without allowing a late worker result to revive the task.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if the task is absent or the action key was reused elsewhere.
    pub fn cancel(
        &mut self,
        task_id: TaskId,
        action_key: &IdempotencyKey,
    ) -> Result<TaskView, StoreError> {
        if let Some((existing_task, _)) = self.action_target(action_key.as_str())? {
            if existing_task != task_id {
                return Err(StoreError::ActionKeyReused(action_key.as_str().to_owned()));
            }
            return self.task(task_id);
        }
        self.task(task_id)?;
        let now = unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        transaction
            .execute(
                "UPDATE tasks SET desired_state = 'cancelled' WHERE id = ?1",
                [task_id.to_string()],
            )
            .map_err(StoreError::Database)?;
        transaction
            .execute(
                "UPDATE attempts SET state = 'cancellation_requested', updated_at = ?1
                 WHERE task_id = ?2 AND state IN ('queued', 'scheduled', 'running')",
                params![now, task_id.to_string()],
            )
            .map_err(StoreError::Database)?;
        insert_action(
            &transaction,
            action_key.as_str(),
            "cancel",
            task_id,
            None,
            now,
        )?;
        transaction.commit().map_err(StoreError::Database)?;
        self.task(task_id)
    }

    /// Records a worker report exactly once and ignores a late report after cancellation.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::AttemptNotFound`] when `attempt_id` is unknown.
    pub fn report(
        &mut self,
        attempt_id: AttemptId,
        report_key: &IdempotencyKey,
        outcome: AttemptOutcome,
    ) -> Result<(), StoreError> {
        let attempt_text = attempt_id.to_string();
        let found = self
            .connection
            .query_row(
                "SELECT task_id, state FROM attempts WHERE id = ?1",
                [&attempt_text],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(StoreError::Database)?
            .ok_or(StoreError::AttemptNotFound(attempt_id))?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO attempt_reports(attempt_id, report_key, outcome, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    attempt_text,
                    report_key.as_str(),
                    outcome_text(outcome),
                    unix_seconds()?
                ],
            )
            .map_err(StoreError::Database)?;
        if inserted == 0 {
            transaction.commit().map_err(StoreError::Database)?;
            return Ok(());
        }
        let desired: String = transaction
            .query_row(
                "SELECT desired_state FROM tasks WHERE id = ?1",
                [&found.0],
                |row| row.get(0),
            )
            .map_err(StoreError::Database)?;
        let current = parse_attempt_state(&found.1)?;
        if desired != "cancelled" && !current.is_terminal() {
            transaction
                .execute(
                    "UPDATE attempts SET state = ?1, updated_at = ?2 WHERE id = ?3",
                    params![
                        attempt_state_text(outcome.into()),
                        unix_seconds()?,
                        attempt_id.to_string()
                    ],
                )
                .map_err(StoreError::Database)?;
        }
        transaction.commit().map_err(StoreError::Database)
    }

    pub(crate) fn reconciliation_targets(&self) -> Result<Vec<ReconciliationTarget>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT a.id, a.task_id, a.ordinal, a.state, a.job_name, t.desired_state,
                    a.worker_image, t.repository_url, t.revision, t.lock_sha256, t.ayni_version
             FROM attempts a JOIN tasks t ON t.id = a.task_id
             WHERE a.state IN ('queued', 'scheduled', 'running', 'cancellation_requested')
             ORDER BY a.created_at ASC",
            )
            .map_err(StoreError::Database)?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            })
            .map_err(StoreError::Database)?
            .map(|row| {
                let (
                    attempt_id,
                    task_id,
                    ordinal,
                    state,
                    job_name,
                    desired,
                    worker_image,
                    repository_url,
                    revision,
                    lock_sha256,
                    ayni_version,
                ) = row.map_err(StoreError::Database)?;
                Ok(ReconciliationTarget {
                    task_id: TaskId::parse(&task_id).map_err(StoreError::InvalidLifecycleData)?,
                    desired_state: parse_desired_state(&desired)?,
                    attempt: AttemptView {
                        id: AttemptId::parse(&attempt_id)
                            .map_err(StoreError::InvalidLifecycleData)?,
                        ordinal,
                        state: parse_attempt_state(&state)?,
                        job_name,
                        worker_image,
                    },
                    input: TaskInput {
                        repository_url,
                        revision,
                        lock_sha256,
                        ayni_version,
                    },
                })
            })
            .collect()
    }

    pub(crate) fn mark_scheduled(
        &self,
        attempt_id: AttemptId,
        job_name: &str,
        worker_image: &str,
    ) -> Result<(), StoreError> {
        self.update_state_if_active(
            attempt_id,
            AttemptState::Scheduled,
            Some(job_name),
            Some(worker_image),
        )
    }

    pub(crate) fn observe(
        &self,
        attempt_id: AttemptId,
        state: AttemptState,
    ) -> Result<(), StoreError> {
        self.update_state_if_active(attempt_id, state, None, None)
    }

    pub(crate) fn mark_cancelled(&self, attempt_id: AttemptId) -> Result<(), StoreError> {
        let updated = self
            .connection
            .execute(
                "UPDATE attempts SET state = 'cancelled', updated_at = ?1
             WHERE id = ?2 AND state = 'cancellation_requested'",
                params![unix_seconds()?, attempt_id.to_string()],
            )
            .map_err(StoreError::Database)?;
        if updated == 0 {
            return Ok(());
        }
        Ok(())
    }

    fn update_state_if_active(
        &self,
        attempt_id: AttemptId,
        next: AttemptState,
        job_name: Option<&str>,
        worker_image: Option<&str>,
    ) -> Result<(), StoreError> {
        let current: Option<String> = self
            .connection
            .query_row(
                "SELECT state FROM attempts WHERE id = ?1",
                [attempt_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::Database)?;
        let Some(current) = current else {
            return Err(StoreError::AttemptNotFound(attempt_id));
        };
        let current = parse_attempt_state(&current)?;
        if current.is_terminal() || current == AttemptState::CancellationRequested {
            return Ok(());
        }
        self.connection.execute(
            "UPDATE attempts SET state = ?1, job_name = COALESCE(?2, job_name), worker_image = COALESCE(?3, worker_image), updated_at = ?4 WHERE id = ?5",
            params![attempt_state_text(next), job_name, worker_image, unix_seconds()?, attempt_id.to_string()],
        ).map_err(StoreError::Database)?;
        Ok(())
    }

    fn action_target(
        &self,
        action_key: &str,
    ) -> Result<Option<(TaskId, Option<AttemptId>)>, StoreError> {
        self.connection
            .query_row(
                "SELECT task_id, attempt_id FROM external_actions WHERE action_key = ?1",
                [action_key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(StoreError::Database)?
            .map(|(task_id, attempt_id)| {
                Ok((
                    TaskId::parse(&task_id).map_err(StoreError::InvalidLifecycleData)?,
                    attempt_id
                        .map(|id| AttemptId::parse(&id).map_err(StoreError::InvalidLifecycleData))
                        .transpose()?,
                ))
            })
            .transpose()
    }
}

include!("orchestration_persistence.rs");
include!("orchestration_transport.rs");
#[cfg(test)]
include!("orchestration_tests.rs");
