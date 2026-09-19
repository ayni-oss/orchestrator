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
    connection: Connection,
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

    fn from_connection(connection: Connection, max_attempts: u32) -> Result<Self, StoreError> {
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
        Ok(Self {
            connection,
            max_attempts,
        })
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
        if task.desired_state == TaskDesiredState::Cancelled {
            return Err(StoreError::TaskCancelled(task_id));
        }
        let latest = task
            .attempts
            .last()
            .ok_or(StoreError::MissingAttempt(task_id))?;
        if !matches!(
            latest.state,
            AttemptState::Failed | AttemptState::Interrupted
        ) {
            return Err(StoreError::RetryNotAllowed(latest.state));
        }
        if latest.ordinal >= task.max_attempts {
            return Err(StoreError::MaxAttemptsReached {
                max_attempts: task.max_attempts,
            });
        }
        let ordinal = latest
            .ordinal
            .checked_add(1)
            .ok_or(StoreError::AttemptOrdinalOverflow)?;
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

fn insert_task(
    transaction: &Transaction<'_>,
    id: TaskId,
    input: &TaskInput,
    max_attempts: u32,
    now: i64,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO tasks(id, repository_url, revision, lock_sha256, ayni_version, desired_state, max_attempts, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7)",
        params![id.to_string(), input.repository_url, input.revision, input.lock_sha256, input.ayni_version, max_attempts, now],
    ).map_err(StoreError::Database)?;
    Ok(())
}

fn migrate_task_max_attempts(connection: &Connection) -> Result<(), StoreError> {
    let mut statement = connection
        .prepare("PRAGMA table_info(tasks)")
        .map_err(StoreError::Database)?;
    let has_column = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(StoreError::Database)?
        .try_fold(
            false,
            |found, column| Ok(found || column? == "max_attempts"),
        )
        .map_err(StoreError::Database)?;
    if !has_column {
        connection
            .execute_batch(
                "ALTER TABLE tasks ADD COLUMN max_attempts INTEGER NOT NULL DEFAULT 3 CHECK(max_attempts > 0);",
            )
            .map_err(StoreError::Database)?;
    }
    Ok(())
}

fn insert_attempt(
    transaction: &Transaction<'_>,
    id: AttemptId,
    task_id: TaskId,
    ordinal: u32,
    state: AttemptState,
    now: i64,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO attempts(id, task_id, ordinal, state, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![id.to_string(), task_id.to_string(), ordinal, attempt_state_text(state), now],
    ).map_err(StoreError::Database)?;
    Ok(())
}

fn insert_action(
    transaction: &Transaction<'_>,
    key: &str,
    kind: &str,
    task_id: TaskId,
    attempt_id: Option<AttemptId>,
    now: i64,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO external_actions(action_key, action_kind, task_id, attempt_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![key, kind, task_id.to_string(), attempt_id.map(|id| id.to_string()), now],
    ).map_err(StoreError::Database)?;
    Ok(())
}

fn unix_seconds() -> Result<i64, StoreError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(StoreError::ClockBeforeEpoch)
        .and_then(|duration| {
            i64::try_from(duration.as_secs()).map_err(|_| StoreError::ClockOverflow)
        })
}

fn parse_desired_state(value: &str) -> Result<TaskDesiredState, StoreError> {
    match value {
        "active" => Ok(TaskDesiredState::Active),
        "cancelled" => Ok(TaskDesiredState::Cancelled),
        _ => Err(StoreError::UnknownDesiredState(value.to_owned())),
    }
}
fn parse_attempt_state(value: &str) -> Result<AttemptState, StoreError> {
    match value {
        "queued" => Ok(AttemptState::Queued),
        "scheduled" => Ok(AttemptState::Scheduled),
        "running" => Ok(AttemptState::Running),
        "succeeded" => Ok(AttemptState::Succeeded),
        "failed" => Ok(AttemptState::Failed),
        "cancellation_requested" => Ok(AttemptState::CancellationRequested),
        "cancelled" => Ok(AttemptState::Cancelled),
        "interrupted" => Ok(AttemptState::Interrupted),
        _ => Err(StoreError::UnknownAttemptState(value.to_owned())),
    }
}
const fn attempt_state_text(value: AttemptState) -> &'static str {
    match value {
        AttemptState::Queued => "queued",
        AttemptState::Scheduled => "scheduled",
        AttemptState::Running => "running",
        AttemptState::Succeeded => "succeeded",
        AttemptState::Failed => "failed",
        AttemptState::CancellationRequested => "cancellation_requested",
        AttemptState::Cancelled => "cancelled",
        AttemptState::Interrupted => "interrupted",
    }
}
const fn outcome_text(value: AttemptOutcome) -> &'static str {
    match value {
        AttemptOutcome::Succeeded => "succeeded",
        AttemptOutcome::Failed => "failed",
    }
}

/// The Kubernetes observation for an attempt-owned Job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobObservation {
    Pending,
    Running,
    Succeeded,
    Failed,
    Missing,
}

/// Creates, observes, and deletes only deterministic attempt-owned Jobs.
pub trait JobTransport: Clone + Send + Sync + 'static {
    /// Returns the immutable worker image identity used for newly scheduled attempts.
    fn worker_image(&self) -> &str;
    /// Ensures an attempt's deterministic Job exists.
    fn ensure_job(
        &self,
        target: &ReconciliationTarget,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>>;
    /// Reads an attempt-owned Job state.
    fn observe_job(
        &self,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<JobObservation, TransportError>> + Send + '_>>;
    /// Requests deletion for an attempt-owned Job.
    fn delete_job(
        &self,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>>;
}

/// Reconciles persistent attempt state to Kubernetes Jobs.
#[derive(Clone)]
pub struct Reconciler<T> {
    store: StoreHandle,
    transport: T,
}

impl<T: JobTransport> Reconciler<T> {
    /// Builds a reconciler around one durable store and transport.
    #[must_use]
    pub const fn new(store: StoreHandle, transport: T) -> Self {
        Self { store, transport }
    }

    /// Reconciles every non-terminal attempt once.
    pub async fn reconcile_once(&self) {
        let targets = match with_store(&self.store, |store| store.reconciliation_targets()) {
            Ok(targets) => targets,
            Err(error) => {
                warn!(%error, "could not load reconciliation targets");
                return;
            }
        };
        for target in targets {
            if let Err(error) = self.reconcile_target(target).await {
                warn!(%error, "attempt reconciliation failed");
            }
        }
    }

    async fn reconcile_target(&self, target: ReconciliationTarget) -> Result<(), StoreError> {
        if target.attempt.state == AttemptState::Queued
            && target.desired_state == TaskDesiredState::Active
        {
            let job_name = job_name(target.attempt.id);
            self.transport
                .ensure_job(&target, &job_name)
                .await
                .map_err(StoreError::Transport)?;
            with_store(&self.store, |store| {
                store.mark_scheduled(target.attempt.id, &job_name, self.transport.worker_image())
            })?;
            info!(task_id = %target.task_id, attempt_id = %target.attempt.id, job_name, "scheduled attempt job");
            return Ok(());
        }
        if target.attempt.state == AttemptState::CancellationRequested {
            let Some(job_name) = target.attempt.job_name.as_deref() else {
                return with_store(&self.store, |store| store.mark_cancelled(target.attempt.id));
            };
            self.transport
                .delete_job(job_name)
                .await
                .map_err(StoreError::Transport)?;
            if self
                .transport
                .observe_job(job_name)
                .await
                .map_err(StoreError::Transport)?
                == JobObservation::Missing
            {
                with_store(&self.store, |store| store.mark_cancelled(target.attempt.id))?;
            }
            return Ok(());
        }
        let Some(job_name) = target.attempt.job_name.as_deref() else {
            return Ok(());
        };
        let state = match self
            .transport
            .observe_job(job_name)
            .await
            .map_err(StoreError::Transport)?
        {
            JobObservation::Pending => AttemptState::Scheduled,
            JobObservation::Running => AttemptState::Running,
            JobObservation::Succeeded => AttemptState::Succeeded,
            JobObservation::Failed => AttemptState::Failed,
            JobObservation::Missing => AttemptState::Interrupted,
        };
        with_store(&self.store, |store| store.observe(target.attempt.id, state))
    }
}

/// Kubernetes API transport using the control-plane service account.
#[derive(Clone)]
pub struct KubernetesJobTransport {
    client: Client,
    namespace: String,
    worker_image: String,
}

impl KubernetesJobTransport {
    /// Builds a transport scoped to one namespace.
    #[must_use]
    pub const fn new(client: Client, namespace: String, worker_image: String) -> Self {
        Self {
            client,
            namespace,
            worker_image,
        }
    }

    fn jobs(&self) -> Api<Job> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }
}

impl JobTransport for KubernetesJobTransport {
    fn worker_image(&self) -> &str {
        &self.worker_image
    }

    fn ensure_job(
        &self,
        target: &ReconciliationTarget,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
        let target = target.clone();
        let job_name = job_name.to_owned();
        Box::pin(async move {
            let job = managed_check_job(&target, &job_name, &self.worker_image)?;
            match self.jobs().create(&PostParams::default(), &job).await {
                Ok(_) => Ok(()),
                Err(create_error) => match self.jobs().get_opt(&job_name).await {
                    Ok(Some(_)) => Ok(()),
                    Ok(None) => Err(TransportError::Kubernetes(create_error.to_string())),
                    Err(get_error) => Err(TransportError::Kubernetes(get_error.to_string())),
                },
            }
        })
    }

    fn observe_job(
        &self,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<JobObservation, TransportError>> + Send + '_>> {
        let job_name = job_name.to_owned();
        Box::pin(async move {
            let Some(job) = self
                .jobs()
                .get_opt(&job_name)
                .await
                .map_err(|error| TransportError::Kubernetes(error.to_string()))?
            else {
                return Ok(JobObservation::Missing);
            };
            let status = job.status;
            if status
                .as_ref()
                .and_then(|status| status.succeeded)
                .unwrap_or_default()
                > 0
            {
                return Ok(JobObservation::Succeeded);
            }
            if status
                .as_ref()
                .and_then(|status| status.failed)
                .unwrap_or_default()
                > 0
            {
                return Ok(JobObservation::Failed);
            }
            if status
                .as_ref()
                .and_then(|status| status.active)
                .unwrap_or_default()
                > 0
            {
                Ok(JobObservation::Running)
            } else {
                Ok(JobObservation::Pending)
            }
        })
    }

    fn delete_job(
        &self,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
        let job_name = job_name.to_owned();
        Box::pin(async move {
            if self
                .jobs()
                .get_opt(&job_name)
                .await
                .map_err(|error| TransportError::Kubernetes(error.to_string()))?
                .is_some()
            {
                self.jobs()
                    .delete(&job_name, &DeleteParams::default())
                    .await
                    .map_err(|error| TransportError::Kubernetes(error.to_string()))?;
            }
            Ok(())
        })
    }
}

fn managed_check_job(
    target: &ReconciliationTarget,
    name: &str,
    worker_image: &str,
) -> Result<Job, TransportError> {
    serde_json::from_value(json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": { "name": name, "labels": { "app.kubernetes.io/name": "ayni-orchestrator-managed-check", "app.kubernetes.io/part-of": "ayni-orchestrator", "orchestrator.ayni.dev/attempt-id": target.attempt.id.to_string() } },
        "spec": {
            "backoffLimit": 0, "activeDeadlineSeconds": 900, "ttlSecondsAfterFinished": 3600,
            "template": { "metadata": { "labels": { "app.kubernetes.io/name": "ayni-orchestrator-managed-check", "orchestrator.ayni.dev/attempt-id": target.attempt.id.to_string() } }, "spec": {
                "restartPolicy": "Never", "automountServiceAccountToken": false,
                "securityContext": { "seccompProfile": { "type": "RuntimeDefault" } },
                "initContainers": [{ "name": "engine", "image": "docker:27.5.1-dind@sha256:aa3df78ecf320f5fafdce71c659f1629e96e9de0968305fe1de670e0ca9176ce", "restartPolicy": "Always", "env": [{ "name": "DOCKER_TLS_CERTDIR", "value": "" }], "securityContext": { "privileged": true }, "resources": { "requests": { "cpu": "4", "memory": "9Gi" }, "limits": { "cpu": "4", "memory": "9Gi" } }, "volumeMounts": [{ "name": "workspace", "mountPath": "/workspace" }, { "name": "engine-storage", "mountPath": "/var/lib/docker" }] }],
                "containers": [{ "name": "worker", "image": worker_image, "imagePullPolicy": "Never", "env": [
                    { "name": "DOCKER_HOST", "value": "tcp://127.0.0.1:2375" }, { "name": "DOCKER_BUILDKIT", "value": "1" }, { "name": "DOCKER_CONFIG", "value": "/tmp/docker" }, { "name": "TMPDIR", "value": "/workspace" }, { "name": "ORCHESTRATOR_ATTEMPT_ID", "value": target.attempt.id.to_string() },
                    { "name": "ORCHESTRATOR_REPOSITORY_URL", "value": target.input.repository_url }, { "name": "ORCHESTRATOR_REVISION", "value": target.input.revision },
                    { "name": "ORCHESTRATOR_LOCK_SHA256", "value": target.input.lock_sha256 }, { "name": "ORCHESTRATOR_AYNI_VERSION", "value": target.input.ayni_version },
                    { "name": "ORCHESTRATOR_WORKSPACE", "value": "/workspace/repository" }, { "name": "ORCHESTRATOR_EVIDENCE_ROOT", "value": "/evidence" }
                ], "securityContext": { "allowPrivilegeEscalation": false, "capabilities": { "drop": ["ALL"] }, "readOnlyRootFilesystem": true, "runAsNonRoot": true }, "resources": { "requests": { "cpu": "250m", "memory": "256Mi" }, "limits": { "cpu": "500m", "memory": "512Mi" } }, "volumeMounts": [{ "name": "workspace", "mountPath": "/workspace" }, { "name": "evidence", "mountPath": "/evidence" }, { "name": "scratch", "mountPath": "/tmp" }] }],
                "volumes": [{ "name": "workspace", "emptyDir": {} }, { "name": "evidence", "persistentVolumeClaim": { "claimName": "managed-check-evidence" } }, { "name": "scratch", "emptyDir": {} }, { "name": "engine-storage", "emptyDir": { "sizeLimit": "20Gi" } }]
            }}
        }
    })).map_err(|error| TransportError::Kubernetes(format!("invalid static Job template: {error}")))
}

fn job_name(attempt_id: AttemptId) -> String {
    format!(
        "managed-check-{}",
        attempt_id.to_string().replace('-', "")[..20].to_owned()
    )
}

fn with_store<T>(
    handle: &StoreHandle,
    operation: impl FnOnce(&mut Store) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let mut store = handle.lock().map_err(|_| StoreError::StoreLockPoisoned)?;
    operation(&mut store)
}

/// A durable store or reconciliation operation failed.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The persistent store directory could not be created.
    #[error("could not create durable store directory")]
    CreateStoreDirectory(#[source] std::io::Error),
    /// `SQLite` rejected an operation.
    #[error("durable store operation failed")]
    Database(#[source] rusqlite::Error),
    /// A persisted value no longer satisfies the domain schema.
    #[error("persisted lifecycle value is invalid")]
    InvalidLifecycleData(#[source] ayni_orchestrator_domain::LifecycleError),
    /// A submitted immutable identity was malformed.
    #[error("task input identity is invalid")]
    InvalidIdentity(#[source] ayni_orchestrator_domain::IdentityError),
    /// A submitted task field was invalid.
    #[error("invalid task input: {0}")]
    InvalidInput(&'static str),
    /// A requested task was absent.
    #[error("task {0} was not found")]
    TaskNotFound(TaskId),
    /// A requested attempt was absent.
    #[error("attempt {0} was not found")]
    AttemptNotFound(AttemptId),
    /// No attempt existed for a task that needed one.
    #[error("task {0} has no attempts")]
    MissingAttempt(TaskId),
    /// A retry was requested for a non-retryable attempt state.
    #[error("attempt state {0:?} cannot be retried")]
    RetryNotAllowed(AttemptState),
    /// A task has reached its immutable attempt limit.
    #[error("task has reached its maximum of {max_attempts} attempts")]
    MaxAttemptsReached {
        /// The durable attempt limit selected when the task was submitted.
        max_attempts: u32,
    },
    /// The configured per-task attempt limit did not allow even one attempt.
    #[error("maximum attempts must be greater than zero, got {0}")]
    InvalidMaxAttempts(u32),
    /// A cancelled task cannot have new work.
    #[error("task {0} is cancelled")]
    TaskCancelled(TaskId),
    /// The caller reused a key for a different state-changing action.
    #[error("idempotency key was already used for a different task: {0}")]
    ActionKeyReused(String),
    /// An attempt number exceeded the supported range.
    #[error("attempt ordinal overflow")]
    AttemptOrdinalOverflow,
    /// The local clock precedes Unix epoch.
    #[error("system clock precedes Unix epoch")]
    ClockBeforeEpoch(#[source] std::time::SystemTimeError),
    /// The Unix timestamp does not fit the store representation.
    #[error("system clock timestamp overflows durable store representation")]
    ClockOverflow,
    /// A state value was not part of the M2 model.
    #[error("unknown persisted desired state {0:?}")]
    UnknownDesiredState(String),
    /// An attempt state value was not part of the M2 model.
    #[error("unknown persisted attempt state {0:?}")]
    UnknownAttemptState(String),
    /// The process could not lock its one `SQLite` connection.
    #[error("durable store lock was poisoned")]
    StoreLockPoisoned,
    /// Kubernetes communication failed.
    #[error(transparent)]
    Transport(#[from] TransportError),
}

/// A Kubernetes Job operation failed.
#[derive(Debug, Error)]
pub enum TransportError {
    /// The Kubernetes API returned an error.
    #[error("Kubernetes API error: {0}")]
    Kubernetes(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::Mutex};

    fn input() -> TaskInput {
        TaskInput {
            repository_url: "https://github.com/gdurandvadas/ayni.git".to_owned(),
            revision: "42f2605d14aa980d14608a8d2f518bc05fcc47b6".to_owned(),
            lock_sha256: "606972cd98e464c8a8befc5191b770539637a9ef3b872ea3ec52abae92928e4f"
                .to_owned(),
            ayni_version: "0.12.1".to_owned(),
        }
    }
    fn key(value: &str) -> Result<IdempotencyKey, StoreError> {
        IdempotencyKey::try_from(value.to_owned()).map_err(StoreError::InvalidLifecycleData)
    }

    #[test]
    fn submission_is_idempotent_and_persists_the_initial_attempt() -> Result<(), StoreError> {
        let mut store = Store::in_memory()?;
        let input = input();
        let first = store.submit(&key("submit-1")?, &input)?;
        let second = store.submit(&key("submit-1")?, &input)?;
        assert_eq!(first, second);
        assert_eq!(first.attempts.len(), 1);
        assert_eq!(first.attempts[0].state, AttemptState::Queued);
        Ok(())
    }

    #[test]
    fn cancellation_prevents_a_late_success_report_from_reviving_the_attempt()
    -> Result<(), StoreError> {
        let mut store = Store::in_memory()?;
        let input = input();
        let task = store.submit(&key("submit-2")?, &input)?;
        let attempt = task.attempts[0].id;
        store.mark_scheduled(attempt, "managed-check-test", "test/worker:latest")?;
        store.cancel(task.id, &key("cancel-2")?)?;
        store.report(attempt, &key("report-2")?, AttemptOutcome::Succeeded)?;
        let updated = store.task(task.id)?;
        assert_eq!(
            updated.attempts[0].state,
            AttemptState::CancellationRequested
        );
        Ok(())
    }

    #[test]
    fn duplicate_report_delivery_records_one_effect() -> Result<(), StoreError> {
        let mut store = Store::in_memory()?;
        let task = store.submit(&key("submit-report")?, &input())?;
        let attempt = task.attempts[0].id;
        let report_key = key("report-duplicate")?;
        store.report(attempt, &report_key, AttemptOutcome::Failed)?;
        store.report(attempt, &report_key, AttemptOutcome::Failed)?;

        let reports: u32 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM attempt_reports WHERE attempt_id = ?1",
                [attempt.to_string()],
                |row| row.get(0),
            )
            .map_err(StoreError::Database)?;
        assert_eq!(reports, 1);
        assert_eq!(store.task(task.id)?.attempts[0].state, AttemptState::Failed);
        Ok(())
    }

    #[test]
    fn retry_limit_is_persisted_with_the_task() -> Result<(), StoreError> {
        let mut store = Store::in_memory_with_max_attempts(2)?;
        let task = store.submit(&key("submit-retry-limit")?, &input())?;
        assert_eq!(task.max_attempts, 2);
        let first_attempt = task.attempts[0].id;
        store.report(
            first_attempt,
            &key("report-first-failure")?,
            AttemptOutcome::Failed,
        )?;
        let retried = store.retry(task.id, &key("retry-once")?)?;
        assert_eq!(retried.attempts.len(), 2);
        let second_attempt = retried.attempts[1].id;
        store.report(
            second_attempt,
            &key("report-second-failure")?,
            AttemptOutcome::Failed,
        )?;

        let Err(error) = store.retry(task.id, &key("retry-beyond-limit")?) else {
            return Err(StoreError::InvalidInput("retry limit was not enforced"));
        };
        assert!(matches!(
            error,
            StoreError::MaxAttemptsReached { max_attempts: 2 }
        ));
        Ok(())
    }

    #[test]
    fn opening_a_legacy_database_adds_the_attempt_limit_column() -> Result<(), StoreError> {
        let connection = Connection::open_in_memory().map_err(StoreError::Database)?;
        connection
            .execute_batch(
                "CREATE TABLE tasks (
                    id TEXT PRIMARY KEY NOT NULL,
                    repository_url TEXT NOT NULL,
                    revision TEXT NOT NULL,
                    lock_sha256 TEXT NOT NULL,
                    ayni_version TEXT NOT NULL,
                    desired_state TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );",
            )
            .map_err(StoreError::Database)?;
        let store = Store::from_connection(connection, 2)?;
        let has_column: bool = store
            .connection
            .prepare("PRAGMA table_info(tasks)")
            .map_err(StoreError::Database)?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(StoreError::Database)?
            .try_fold(
                false,
                |found, column| Ok(found || column? == "max_attempts"),
            )
            .map_err(StoreError::Database)?;
        assert!(has_column);
        Ok(())
    }

    #[derive(Clone, Default)]
    struct FakeTransport {
        observations: Arc<Mutex<BTreeMap<String, JobObservation>>>,
    }
    impl JobTransport for FakeTransport {
        fn worker_image(&self) -> &'static str {
            "test/worker@sha256:0000000000000000000000000000000000000000000000000000000000000000"
        }

        fn ensure_job(
            &self,
            _target: &ReconciliationTarget,
            name: &str,
        ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
            let name = name.to_owned();
            let observations = Arc::clone(&self.observations);
            Box::pin(async move {
                observations
                    .lock()
                    .map_err(|_| TransportError::Kubernetes("test lock".to_owned()))?
                    .insert(name, JobObservation::Pending);
                Ok(())
            })
        }
        fn observe_job(
            &self,
            name: &str,
        ) -> Pin<Box<dyn Future<Output = Result<JobObservation, TransportError>> + Send + '_>>
        {
            let name = name.to_owned();
            let observations = Arc::clone(&self.observations);
            Box::pin(async move {
                Ok(*observations
                    .lock()
                    .map_err(|_| TransportError::Kubernetes("test lock".to_owned()))?
                    .get(&name)
                    .unwrap_or(&JobObservation::Missing))
            })
        }
        fn delete_job(
            &self,
            name: &str,
        ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
            let name = name.to_owned();
            let observations = Arc::clone(&self.observations);
            Box::pin(async move {
                observations
                    .lock()
                    .map_err(|_| TransportError::Kubernetes("test lock".to_owned()))?
                    .insert(name, JobObservation::Missing);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn reconciliation_recovers_after_a_restart_without_creating_a_second_job()
    -> Result<(), StoreError> {
        let store = Arc::new(Mutex::new(Store::in_memory()?));
        let input = input();
        let task = with_store(&store, |store| store.submit(&key("submit-3")?, &input))?;
        let transport = FakeTransport::default();
        let reconciler = Reconciler::new(Arc::clone(&store), transport.clone());
        reconciler.reconcile_once().await;
        reconciler.reconcile_once().await;
        let task = with_store(&store, |store| store.task(task.id))?;
        assert_eq!(task.attempts[0].state, AttemptState::Scheduled);
        assert_eq!(
            transport
                .observations
                .lock()
                .map_err(|_| StoreError::StoreLockPoisoned)?
                .len(),
            1
        );
        Ok(())
    }
}
