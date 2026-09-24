fn retry_ordinal(task: &TaskView) -> Result<u32, StoreError> {
    if task.desired_state == TaskDesiredState::Cancelled {
        return Err(StoreError::TaskCancelled(task.id));
    }
    let latest = task.attempts.last().ok_or(StoreError::MissingAttempt(task.id))?;
    if !matches!(latest.state, AttemptState::Failed | AttemptState::Interrupted) {
        return Err(StoreError::RetryNotAllowed(latest.state));
    }
    if latest.ordinal >= task.max_attempts {
        return Err(StoreError::MaxAttemptsReached { max_attempts: task.max_attempts });
    }
    latest.ordinal.checked_add(1).ok_or(StoreError::AttemptOrdinalOverflow)
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

/// Returns current Unix seconds for durable transition timestamps.
///
/// # Errors
///
/// Returns an error if the local clock precedes the epoch or overflows `SQLite`'s range.
pub fn unix_seconds() -> Result<i64, StoreError> {
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
