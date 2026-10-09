use super::*;

pub(super) fn is_terminal_attempt(state: i32) -> bool {
    matches!(
        v1::DevelopmentAttemptState::try_from(state).ok(),
        Some(
            v1::DevelopmentAttemptState::Succeeded
                | v1::DevelopmentAttemptState::Failed
                | v1::DevelopmentAttemptState::Cancelled
                | v1::DevelopmentAttemptState::DeadlineExceeded
                | v1::DevelopmentAttemptState::Unsupported
        )
    )
}

pub(super) const fn valid_repository_transition(
    current: v1::RepositoryConnectionStatus,
    next: v1::RepositoryConnectionStatus,
) -> bool {
    use v1::RepositoryConnectionStatus as State;
    matches!(
        (current, next),
        (
            State::Connecting,
            State::Connected | State::Degraded | State::Revoked | State::Failed
        ) | (State::Connected, State::Degraded | State::Revoked)
            | (
                State::Degraded,
                State::Connecting | State::Connected | State::Revoked | State::Failed
            )
    )
}

pub(super) const fn valid_profile_transition(
    current: v1::RuntimeProfileState,
    next: v1::RuntimeProfileState,
) -> bool {
    use v1::RuntimeProfileState as State;
    matches!(
        (current, next),
        (
            State::SetupRequired,
            State::SetupPrOpen | State::AwaitingEnvironment | State::Degraded | State::Failed
        ) | (
            State::SetupPrOpen,
            State::SetupRequired | State::AwaitingEnvironment | State::Degraded | State::Failed
        ) | (
            State::AwaitingEnvironment,
            State::SetupRequired | State::Ready | State::Updating | State::Degraded | State::Failed
        ) | (
            State::Ready,
            State::SetupRequired | State::Updating | State::Degraded
        ) | (
            State::Updating,
            State::SetupRequired | State::Ready | State::Degraded | State::Failed
        ) | (
            State::Degraded,
            State::SetupRequired
                | State::AwaitingEnvironment
                | State::Updating
                | State::Ready
                | State::Failed
        )
    )
}

#[cfg(test)]
pub(super) const fn valid_artifact_transition(
    current: v1::RuntimeArtifactStatus,
    next: v1::RuntimeArtifactStatus,
) -> bool {
    use v1::RuntimeArtifactStatus as State;
    matches!(
        (current, next),
        (
            State::Discovered,
            State::Verifying | State::Rejected | State::Unavailable | State::Failed
        ) | (
            State::Verifying,
            State::Verified | State::Rejected | State::Unavailable | State::Failed
        ) | (State::Verified, State::Unavailable)
            | (State::Unavailable, State::Verifying | State::Failed)
    )
}

pub(super) const fn valid_attempt_transition(
    current: v1::DevelopmentAttemptState,
    next: v1::DevelopmentAttemptState,
) -> bool {
    use v1::DevelopmentAttemptState as State;
    matches!(
        (current, next),
        (
            State::Queued,
            State::Running | State::Cancelled | State::DeadlineExceeded | State::Unsupported
        ) | (
            State::Running,
            State::WaitingForInput
                | State::Succeeded
                | State::Failed
                | State::Cancelled
                | State::DeadlineExceeded
        ) | (
            State::WaitingForInput,
            State::Running | State::Failed | State::Cancelled | State::DeadlineExceeded
        )
    )
}

pub(super) const fn task_state_for_attempt(
    state: v1::DevelopmentAttemptState,
) -> v1::DevelopmentTaskState {
    match state {
        v1::DevelopmentAttemptState::Queued => v1::DevelopmentTaskState::Queued,
        v1::DevelopmentAttemptState::Running | v1::DevelopmentAttemptState::WaitingForInput => {
            v1::DevelopmentTaskState::Running
        }
        v1::DevelopmentAttemptState::Succeeded => v1::DevelopmentTaskState::Succeeded,
        v1::DevelopmentAttemptState::Failed => v1::DevelopmentTaskState::Failed,
        v1::DevelopmentAttemptState::Cancelled => v1::DevelopmentTaskState::Cancelled,
        v1::DevelopmentAttemptState::DeadlineExceeded => v1::DevelopmentTaskState::DeadlineExceeded,
        v1::DevelopmentAttemptState::Unsupported | v1::DevelopmentAttemptState::Unspecified => {
            v1::DevelopmentTaskState::Unsupported
        }
    }
}

pub(super) const fn map_constraint(error: rusqlite::Error) -> CleanStoreError {
    CleanStoreError::Database(error)
}

#[derive(Debug)]
pub(super) struct ActionRow {
    pub(super) request_hash: Vec<u8>,
    pub(super) request_payload: Vec<u8>,
    pub(super) state: String,
    pub(super) target_id: Option<String>,
}

impl ActionRow {
    pub(super) fn into_action(self, key: &str) -> DurableAction {
        let state = match self.state.as_str() {
            "completed" => DurableActionState::Completed,
            "failed" => DurableActionState::Failed,
            _ => DurableActionState::Pending,
        };
        DurableAction {
            idempotency_key: key.to_owned(),
            state,
            request_payload: self.request_payload,
        }
    }
}

pub(super) fn action_row(
    connection: &Connection,
    operation: &str,
    key: &str,
) -> Result<Option<ActionRow>, CleanStoreError> {
    connection
        .query_row(
            "SELECT request_hash,request_payload,state,target_id FROM durable_actions
             WHERE operation=?1 AND idempotency_key=?2",
            params![operation, key],
            |row| {
                Ok(ActionRow {
                    request_hash: row.get(0)?,
                    request_payload: row.get(1)?,
                    state: row.get(2)?,
                    target_id: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(CleanStoreError::Database)
}

pub(crate) fn request_digest<M: Message>(operation: &str, request: &M) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(b"ayni.orchestrator.action.v1\0");
    hasher.update(operation.as_bytes());
    hasher.update([0]);
    hasher.update(request.encode_to_vec());
    hasher.finalize().to_vec()
}

pub(super) fn initialize_schema(connection: &Connection) -> Result<(), CleanStoreError> {
    let tables = existing_tables(connection)?;
    let has_marker = tables.iter().any(|table| table == "clean_schema");
    if !tables.is_empty() && !has_marker {
        return Err(CleanStoreError::RetiredPilotSchema);
    }
    if has_marker {
        let mut version: i64 = connection
            .query_row(
                "SELECT version FROM clean_schema WHERE component='control_plane'",
                [],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        if version == 6 {
            let expected = CLEAN_TABLES
                .iter()
                .filter(|name| !matches!(**name, "certificates" | "certificate_operations"))
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            let mut expected = expected;
            expected.push("automation_installations".into());
            expected.sort();
            if tables != expected {
                return Err(CleanStoreError::CorruptState);
            }
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE certificates(id TEXT PRIMARY KEY, is_default INTEGER NOT NULL CHECK(is_default IN (0,1)), payload BLOB NOT NULL); CREATE UNIQUE INDEX single_default_certificate ON certificates(is_default) WHERE is_default=1; CREATE TABLE certificate_operations(operation_key TEXT PRIMARY KEY, certificate_id TEXT NOT NULL, operation TEXT NOT NULL, revision INTEGER NOT NULL, fingerprint BLOB NOT NULL); UPDATE clean_schema SET version=7 WHERE component='control_plane'; COMMIT;").map_err(CleanStoreError::Database)?;
            version = 7;
        }
        if version == 7 {
            let mut expected = CLEAN_TABLES
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            expected.push("automation_installations".into());
            expected.sort();
            if existing_tables(connection)? != expected {
                return Err(CleanStoreError::CorruptState);
            }
            connection.execute_batch("BEGIN IMMEDIATE; DROP TABLE automation_installations; DELETE FROM durable_actions WHERE operation='InstallRepositoryAutomation'; DELETE FROM resource_events WHERE resource_type='automation_installation'; DELETE FROM page_snapshots; UPDATE reconciliation_cursors SET value=json_set(value,'$.pending',null,'$.next_at',0,'$.failures',0,'$.status',json('{}')) WHERE name LIKE 'managed:%' AND json_valid(value); UPDATE clean_schema SET version=8 WHERE component='control_plane'; COMMIT;").map_err(CleanStoreError::Database)?;
            version = 8;
        }
        if version == 8 {
            super::builder_cutover::migrate(connection)?;
            version = 9;
        }
        if version != SCHEMA_VERSION {
            return Err(CleanStoreError::UnsupportedSchemaVersion(version));
        }
        if existing_tables(connection)?
            != CLEAN_TABLES
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        {
            return Err(CleanStoreError::CorruptState);
        }
        return Ok(());
    }
    connection
        .execute_batch(CLEAN_SCHEMA)
        .map_err(CleanStoreError::Database)?;
    Ok(())
}

pub(super) fn existing_tables(connection: &Connection) -> Result<Vec<String>, CleanStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(CleanStoreError::Database)?;
    statement
        .query_map([], |row| row.get(0))
        .map_err(CleanStoreError::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(CleanStoreError::Database)
}

pub(super) const CLEAN_SCHEMA: &str = r"
BEGIN IMMEDIATE;
CREATE TABLE clean_schema(
    component TEXT PRIMARY KEY CHECK(component='control_plane'),
    version INTEGER NOT NULL CHECK(version > 0),
    created_at INTEGER NOT NULL
);
INSERT INTO clean_schema(component,version,created_at)
VALUES ('control_plane',9,unixepoch());
CREATE TABLE certificates(id TEXT PRIMARY KEY, is_default INTEGER NOT NULL CHECK(is_default IN (0,1)), payload BLOB NOT NULL);
CREATE UNIQUE INDEX single_default_certificate ON certificates(is_default) WHERE is_default=1;
CREATE TABLE certificate_operations(operation_key TEXT PRIMARY KEY, certificate_id TEXT NOT NULL, operation TEXT NOT NULL, revision INTEGER NOT NULL, fingerprint BLOB NOT NULL);
CREATE TABLE connections(id TEXT PRIMARY KEY, kind TEXT NOT NULL, payload BLOB NOT NULL);
CREATE UNIQUE INDEX single_github_app ON connections(kind) WHERE kind='github_app';
CREATE TABLE connection_operations(operation_key TEXT PRIMARY KEY, connection_id TEXT NOT NULL, operation TEXT NOT NULL, revision INTEGER NOT NULL);

CREATE TABLE repository_connections(
    id TEXT PRIMARY KEY, status INTEGER NOT NULL, provider_key TEXT NOT NULL UNIQUE,
    payload BLOB NOT NULL, resource_version INTEGER NOT NULL CHECK(resource_version > 0),
    event_cursor INTEGER NOT NULL UNIQUE
);
CREATE TABLE projects(
    id TEXT PRIMARY KEY, state INTEGER NOT NULL CHECK(state IN (1,2)),
    payload BLOB NOT NULL, resource_version INTEGER NOT NULL CHECK(resource_version > 0),
    event_cursor INTEGER NOT NULL UNIQUE
);
CREATE TABLE project_repositories(
    project_id TEXT NOT NULL REFERENCES projects(id),
    repository_connection_id TEXT NOT NULL REFERENCES repository_connections(id),
    active INTEGER NOT NULL CHECK(active IN (0,1)),
    PRIMARY KEY(project_id,repository_connection_id)
);
CREATE INDEX active_project_repository ON project_repositories(repository_connection_id) WHERE active=1;
CREATE TABLE repository_snapshots(
    id TEXT PRIMARY KEY,
    repository_connection_id TEXT NOT NULL REFERENCES repository_connections(id),
    revision TEXT NOT NULL, payload BLOB NOT NULL,
    resource_version INTEGER NOT NULL CHECK(resource_version > 0), event_cursor INTEGER NOT NULL UNIQUE,
    UNIQUE(repository_connection_id,revision)
);
CREATE TABLE runtime_profiles(
    id TEXT PRIMARY KEY,
    repository_connection_id TEXT NOT NULL REFERENCES repository_connections(id),
    kind INTEGER NOT NULL, state INTEGER NOT NULL, active_revision_id TEXT,
    active_runtime_artifact_id TEXT, payload BLOB NOT NULL,
    resource_version INTEGER NOT NULL CHECK(resource_version > 0), event_cursor INTEGER NOT NULL UNIQUE
);
CREATE TABLE runtime_profile_revisions(
    id TEXT PRIMARY KEY,
    runtime_profile_id TEXT NOT NULL REFERENCES runtime_profiles(id),
    repository_snapshot_id TEXT NOT NULL REFERENCES repository_snapshots(id),
    revision_number INTEGER NOT NULL CHECK(revision_number > 0), payload BLOB NOT NULL,
    resource_version INTEGER NOT NULL CHECK(resource_version > 0), event_cursor INTEGER NOT NULL UNIQUE,
    UNIQUE(runtime_profile_id,revision_number)
);
CREATE TABLE runtime_artifacts(
    id TEXT PRIMARY KEY,
    runtime_profile_id TEXT NOT NULL REFERENCES runtime_profiles(id),
    runtime_profile_revision_id TEXT NOT NULL REFERENCES runtime_profile_revisions(id),
    status INTEGER NOT NULL, resolved_digest TEXT NOT NULL, platform_key TEXT NOT NULL,
    payload BLOB NOT NULL, resource_version INTEGER NOT NULL CHECK(resource_version > 0),
    event_cursor INTEGER NOT NULL UNIQUE,
    UNIQUE(runtime_profile_revision_id,resolved_digest,platform_key)
);
CREATE TABLE development_tasks(
    project_id TEXT NOT NULL REFERENCES projects(id),
    id TEXT PRIMARY KEY,
    repository_connection_id TEXT NOT NULL REFERENCES repository_connections(id),
    runtime_profile_id TEXT NOT NULL REFERENCES runtime_profiles(id),
    state INTEGER NOT NULL, active_attempt_id TEXT,
    deadline_at_seconds INTEGER, deadline_at_nanos INTEGER,
    payload BLOB NOT NULL,
    resource_version INTEGER NOT NULL CHECK(resource_version > 0), event_cursor INTEGER NOT NULL UNIQUE
);
CREATE INDEX development_tasks_deadline_idx
ON development_tasks(state,deadline_at_seconds,deadline_at_nanos);
CREATE TABLE development_attempts(
    id TEXT PRIMARY KEY,
    development_task_id TEXT NOT NULL REFERENCES development_tasks(id),
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0), state INTEGER NOT NULL,
    payload BLOB NOT NULL, resource_version INTEGER NOT NULL CHECK(resource_version > 0),
    event_cursor INTEGER NOT NULL UNIQUE,
    UNIQUE(development_task_id,attempt_number)
);
CREATE TABLE durable_actions(
    operation TEXT NOT NULL, idempotency_key TEXT NOT NULL, request_hash BLOB NOT NULL,
    request_payload BLOB NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending','completed','failed')),
    target_type TEXT, target_id TEXT, failure TEXT,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    PRIMARY KEY(operation,idempotency_key)
);
CREATE INDEX durable_actions_state_idx ON durable_actions(operation,state,created_at);
CREATE TABLE resource_events(
    cursor INTEGER PRIMARY KEY AUTOINCREMENT,
    service TEXT NOT NULL, resource_type TEXT NOT NULL, resource_id TEXT NOT NULL,
    resource_version INTEGER NOT NULL, repository_connection_id TEXT, parent_id TEXT,
    payload BLOB NOT NULL, created_at INTEGER NOT NULL
);
CREATE INDEX resource_events_service_cursor_idx ON resource_events(service,cursor);
CREATE INDEX resource_events_repository_idx ON resource_events(service,repository_connection_id,cursor);
CREATE INDEX resource_events_parent_idx ON resource_events(service,parent_id,cursor);
CREATE TABLE event_watermarks(
    service TEXT PRIMARY KEY, first_cursor INTEGER NOT NULL CHECK(first_cursor >= 0)
);
CREATE TABLE reconciliation_cursors(
    name TEXT PRIMARY KEY, value TEXT NOT NULL, resource_version INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE page_snapshots(
    token TEXT PRIMARY KEY, method TEXT NOT NULL, filter_hash TEXT NOT NULL,
    cursor INTEGER NOT NULL, items TEXT NOT NULL, token_secret TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX page_snapshots_created_idx ON page_snapshots(created_at);
COMMIT;
";

pub(super) fn unix_seconds() -> Result<i64, CleanStoreError> {
    unix_timestamp().map(|value| value.seconds)
}

pub(super) fn unix_timestamp() -> Result<Timestamp, CleanStoreError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CleanStoreError::Clock)?;
    Ok(Timestamp {
        seconds: i64::try_from(duration.as_secs()).map_err(|_| CleanStoreError::Clock)?,
        nanos: i32::try_from(duration.subsec_nanos()).map_err(|_| CleanStoreError::Clock)?,
    })
}

#[cfg(test)]
pub(super) fn add_duration(
    base: &Timestamp,
    duration: &prost_types::Duration,
) -> Result<Timestamp, CleanStoreError> {
    if duration.seconds < 0 || duration.nanos < 0 || duration.nanos >= 1_000_000_000 {
        return Err(CleanStoreError::InvalidTransition);
    }
    let nanos = i64::from(base.nanos).saturating_add(i64::from(duration.nanos));
    let seconds = base
        .seconds
        .checked_add(duration.seconds)
        .and_then(|value| value.checked_add(nanos / 1_000_000_000))
        .ok_or(CleanStoreError::Clock)?;
    Ok(Timestamp {
        seconds,
        nanos: i32::try_from(nanos % 1_000_000_000).map_err(|_| CleanStoreError::Clock)?,
    })
}

pub(super) const fn timestamp(seconds: i64) -> Timestamp {
    Timestamp { seconds, nanos: 0 }
}

pub(super) fn resource_id(prefix: &str) -> String {
    format!("{prefix}:{}", Uuid::now_v7())
}

pub(super) fn decode<M: Message + Default>(bytes: &[u8]) -> Result<M, CleanStoreError> {
    M::decode(bytes).map_err(|_| CleanStoreError::CorruptState)
}
