//! Durable clean-v1 resource storage.
#![allow(
    dead_code,
    clippy::items_after_test_module,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_pass_by_value,
    clippy::redundant_pub_crate,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::unused_self,
    reason = "shared reconciliation storage keeps explicit resource boundaries and hooks"
)]
//!
//! The clean schema is intentionally unrelated to the retired pilot schema. Public
//! resources are stored as protobuf payloads beside the indexed identities needed
//! for filtering and reconciliation. Every represented mutation appends a durable
//! service event in the same `SQLite` transaction.

use ayni_orchestrator_api::{v1, validation as api_validation};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use prost::Message;
use prost_types::Timestamp;
use rusqlite::{Connection, OptionalExtension, Transaction, params, types::ValueRef};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use uuid::Uuid;

const SCHEMA_VERSION: i64 = 1;
const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: usize = 100;
const PAGE_LIFETIME_SECONDS: i64 = 15 * 60;
const MAX_PAGE_SNAPSHOTS: usize = 128;
const MAX_PAGE_SNAPSHOT_ITEMS: usize = 10_000;
const MAX_PAGE_SNAPSHOT_BYTES: usize = 8 * 1_024 * 1_024;
const MAX_PAGE_RESPONSE_BYTES: usize = 3 * 1_024 * 1_024;
const MAX_EVENTS_PER_SERVICE: usize = 4_096;
const MAX_EVIDENCE_SIZE_BYTES: u64 = 64 * 1_024 * 1_024;
const MAX_RECONCILE_BATCH: usize = 128;
const MAX_PENDING_ACTION_BATCH: usize = 256;
const CLEAN_TABLES: &[&str] = &[
    "assessments",
    "automation_installations",
    "candidates",
    "change_requests",
    "clean_schema",
    "development_attempts",
    "development_tasks",
    "durable_actions",
    "event_watermarks",
    "evidence_references",
    "page_snapshots",
    "reconciliation_cursors",
    "repository_connections",
    "repository_snapshots",
    "resource_events",
    "runtime_artifacts",
    "runtime_profile_revisions",
    "runtime_profiles",
];

pub(crate) const REPOSITORY_SERVICE: &str = "repositories";
pub(crate) const PROFILE_SERVICE: &str = "runtime_profiles";
pub(crate) const DEVELOPMENT_SERVICE: &str = "development_tasks";
pub(crate) const REVIEW_SERVICE: &str = "reviews";

#[derive(Debug, Error)]
pub enum CleanStoreError {
    #[error("could not create the durable-store directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("durable SQLite operation failed")]
    Database(#[source] rusqlite::Error),
    #[error(
        "the database contains the retired pilot schema; back it up if needed, then run the documented guarded purge/reset procedure"
    )]
    RetiredPilotSchema,
    #[error("unsupported clean-v1 schema version {0}")]
    UnsupportedSchemaVersion(i64),
    #[error("durable clean-v1 state is corrupt")]
    CorruptState,
    #[error("resource {resource_type} {resource_id} was not found")]
    NotFound {
        resource_type: &'static str,
        resource_id: String,
    },
    #[error("idempotency key was reused with different request content")]
    IdempotencyConflict,
    #[error("the requested resource transition is not supported")]
    InvalidTransition,
    #[error("the resource changed after it was observed")]
    StaleVersion,
    #[error("a referenced resource does not belong to the requested parent")]
    ParentMismatch,
    #[error("the runtime profile is not ready with a verified artifact")]
    ProfileNotReady,
    #[error("the task is not actionable")]
    TaskNotActionable,
    #[error("the task deadline has expired")]
    DeadlineExpired,
    #[error("the page token is invalid or expired")]
    InvalidPageToken,
    #[error("the bounded result cannot be retained or returned safely")]
    BoundExceeded,
    #[error("the watch cursor has expired")]
    WatchCursorExpired,
    #[error("the clock is before the Unix epoch or outside SQLite range")]
    Clock,
}

#[derive(Clone, Debug)]
pub(crate) struct Page<T> {
    pub items: Vec<T>,
    pub next_page_token: String,
    pub snapshot_cursor: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct StoredEvent {
    pub cursor: u64,
    pub resource_type: String,
    pub resource_id: String,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DurableActionState {
    Pending,
    Completed,
    Failed,
}

#[derive(Clone, Debug)]
pub(crate) struct DurableAction {
    pub operation: String,
    pub idempotency_key: String,
    pub state: DurableActionState,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub request_payload: Vec<u8>,
    pub failure: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PageToken {
    snapshot: String,
    offset: usize,
    proof: String,
}

pub(crate) struct CleanStore {
    connection: Connection,
    max_attempts: u32,
    path: Option<PathBuf>,
}

impl CleanStore {
    pub(crate) fn open(path: impl AsRef<Path>, max_attempts: u32) -> Result<Self, CleanStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(CleanStoreError::CreateDirectory)?;
        }
        let connection = Connection::open(path).map_err(CleanStoreError::Database)?;
        Self::from_connection(connection, max_attempts, Some(path.to_path_buf()))
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Result<Self, CleanStoreError> {
        Self::from_connection(
            Connection::open_in_memory().map_err(CleanStoreError::Database)?,
            3,
            None,
        )
    }

    fn from_connection(
        connection: Connection,
        max_attempts: u32,
        path: Option<PathBuf>,
    ) -> Result<Self, CleanStoreError> {
        if max_attempts == 0 {
            return Err(CleanStoreError::InvalidTransition);
        }
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(CleanStoreError::Database)?;
        initialize_schema(&connection)?;
        let store = Self {
            connection,
            max_attempts,
            path,
        };
        store.integrity_check()?;
        store.foreign_key_check()?;
        store.validate_durable_state()?;
        Ok(store)
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn schema_version(&self) -> Result<i64, CleanStoreError> {
        self.connection
            .query_row(
                "SELECT version FROM clean_schema WHERE component='control_plane'",
                [],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)
    }

    pub(crate) fn integrity_check(&self) -> Result<(), CleanStoreError> {
        let result: String = self
            .connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(CleanStoreError::Database)?;
        if result == "ok" {
            Ok(())
        } else {
            Err(CleanStoreError::CorruptState)
        }
    }

    fn foreign_key_check(&self) -> Result<(), CleanStoreError> {
        let mut statement = self
            .connection
            .prepare("PRAGMA foreign_key_check")
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        if rows.next().map_err(CleanStoreError::Database)?.is_some() {
            Err(CleanStoreError::CorruptState)
        } else {
            Ok(())
        }
    }

    fn validate_durable_state(&self) -> Result<(), CleanStoreError> {
        validate_resource_tables(&self.connection)?;
        validate_durable_actions(&self.connection)?;
        validate_resource_events(&self.connection)?;
        validate_page_snapshot_rows(&self.connection)?;
        Ok(())
    }

    pub(crate) fn current_cursor(&self, service: &str) -> Result<u64, CleanStoreError> {
        let value: i64 = self
            .connection
            .query_row(
                "SELECT COALESCE(MAX(cursor), 0) FROM resource_events WHERE service=?1",
                [service],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        u64::try_from(value).map_err(|_| CleanStoreError::CorruptState)
    }

    pub(crate) fn initial_events(
        &self,
        service: &str,
        repository_connection_id: Option<&str>,
        parent_id: Option<&str>,
    ) -> Result<(Vec<StoredEvent>, u64), CleanStoreError> {
        let (sql, parameters): (&str, Vec<Option<&str>>) = match service {
            REPOSITORY_SERVICE => (
                "SELECT event_cursor,'repository_connection',id,payload FROM repository_connections
                 WHERE (?1 IS NULL OR id=?1)
                 UNION ALL
                 SELECT event_cursor,'repository_snapshot',id,payload FROM repository_snapshots
                 WHERE (?1 IS NULL OR repository_connection_id=?1)
                 ORDER BY 1",
                vec![repository_connection_id],
            ),
            PROFILE_SERVICE => (
                "SELECT event_cursor,'runtime_profile',id,payload FROM runtime_profiles
                 WHERE (?1 IS NULL OR repository_connection_id=?1)
                   AND (?2 IS NULL OR id=?2)
                 UNION ALL
                 SELECT r.event_cursor,'runtime_profile_revision',r.id,r.payload
                 FROM runtime_profile_revisions r JOIN runtime_profiles p ON p.id=r.runtime_profile_id
                 WHERE (?1 IS NULL OR p.repository_connection_id=?1)
                   AND (?2 IS NULL OR p.id=?2)
                 UNION ALL
                 SELECT a.event_cursor,'runtime_artifact',a.id,a.payload
                 FROM runtime_artifacts a JOIN runtime_profiles p ON p.id=a.runtime_profile_id
                 WHERE (?1 IS NULL OR p.repository_connection_id=?1)
                   AND (?2 IS NULL OR p.id=?2)
                 UNION ALL
                 SELECT i.event_cursor,'automation_installation',i.id,i.payload
                 FROM automation_installations i JOIN runtime_profiles p ON p.id=i.runtime_profile_id
                 WHERE (?1 IS NULL OR p.repository_connection_id=?1)
                   AND (?2 IS NULL OR p.id=?2)
                 ORDER BY 1",
                vec![repository_connection_id, parent_id],
            ),
            DEVELOPMENT_SERVICE => (
                "SELECT event_cursor,'development_task',id,payload FROM development_tasks WHERE id=?1
                 UNION ALL
                 SELECT event_cursor,'development_attempt',id,payload FROM development_attempts
                 WHERE development_task_id=?1
                 UNION ALL
                 SELECT event_cursor,'candidate',id,payload FROM candidates
                 WHERE development_task_id=?1
                 ORDER BY 1",
                vec![parent_id],
            ),
            REVIEW_SERVICE => (
                "SELECT event_cursor,'change_request',id,payload FROM change_requests
                 WHERE (?1 IS NULL OR repository_connection_id=?1)
                   AND (?2 IS NULL OR id=?2)
                 UNION ALL
                 SELECT a.event_cursor,'assessment',a.id,a.payload
                 FROM assessments a JOIN change_requests c ON c.id=a.change_request_id
                 WHERE (?1 IS NULL OR c.repository_connection_id=?1)
                   AND (?2 IS NULL OR c.id=?2)
                 UNION ALL
                 SELECT e.event_cursor,'evidence_reference',e.id,e.payload
                 FROM evidence_references e
                 JOIN assessments a ON a.id=e.assessment_id
                 JOIN change_requests c ON c.id=a.change_request_id
                 WHERE (?1 IS NULL OR c.repository_connection_id=?1)
                   AND (?2 IS NULL OR c.id=?2)
                 ORDER BY 1",
                vec![repository_connection_id, parent_id],
            ),
            _ => return Err(CleanStoreError::CorruptState),
        };
        let mut statement = self
            .connection
            .prepare(sql)
            .map_err(CleanStoreError::Database)?;
        let mut rows = match parameters.as_slice() {
            [first] => statement
                .query(params![first])
                .map_err(CleanStoreError::Database)?,
            [first, second] => statement
                .query(params![first, second])
                .map_err(CleanStoreError::Database)?,
            _ => return Err(CleanStoreError::CorruptState),
        };
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
            let cursor: i64 = row.get(0).map_err(CleanStoreError::Database)?;
            events.push(StoredEvent {
                cursor: u64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                resource_type: row.get(1).map_err(CleanStoreError::Database)?,
                resource_id: row.get(2).map_err(CleanStoreError::Database)?,
                payload: row.get(3).map_err(CleanStoreError::Database)?,
            });
        }
        let high_water = self.current_cursor(service)?;
        Ok((events, high_water))
    }

    pub(crate) fn events_after(
        &self,
        service: &str,
        after_cursor: u64,
        repository_connection_id: Option<&str>,
        parent_id: Option<&str>,
    ) -> Result<Vec<StoredEvent>, CleanStoreError> {
        let after = i64::try_from(after_cursor).map_err(|_| CleanStoreError::WatchCursorExpired)?;
        let watermark: i64 = self
            .connection
            .query_row(
                "SELECT first_cursor FROM event_watermarks WHERE service=?1",
                [service],
                |row| row.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?
            .unwrap_or(0);
        let high_water: i64 = self
            .connection
            .query_row(
                "SELECT COALESCE(MAX(cursor),0) FROM resource_events WHERE service=?1",
                [service],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        if after < watermark.saturating_sub(1) || after > high_water {
            return Err(CleanStoreError::WatchCursorExpired);
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT cursor, resource_type, resource_id, payload
                 FROM resource_events
                 WHERE service=?1 AND cursor>?2
                   AND (?3 IS NULL OR repository_connection_id=?3)
                   AND (?4 IS NULL OR parent_id=?4)
                 ORDER BY cursor ASC LIMIT 256",
            )
            .map_err(CleanStoreError::Database)?;
        statement
            .query_map(
                params![service, after, repository_connection_id, parent_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                },
            )
            .map_err(CleanStoreError::Database)?
            .map(|row| {
                let (cursor, resource_type, resource_id, payload) =
                    row.map_err(CleanStoreError::Database)?;
                Ok(StoredEvent {
                    cursor: u64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                    resource_type,
                    resource_id,
                    payload,
                })
            })
            .collect()
    }

    pub(crate) fn durable_action(
        &self,
        operation: &str,
        key: &str,
    ) -> Result<Option<DurableAction>, CleanStoreError> {
        self.connection
            .query_row(
                "SELECT state, target_type, target_id, request_payload, failure FROM durable_actions
                 WHERE operation=?1 AND idempotency_key=?2",
                params![operation, key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(CleanStoreError::Database)?
            .map(|(state, target_type, target_id, request_payload, failure)| {
                let state = match state.as_str() {
                    "pending" => DurableActionState::Pending,
                    "completed" => DurableActionState::Completed,
                    "failed" => DurableActionState::Failed,
                    _ => return Err(CleanStoreError::CorruptState),
                };
                Ok(DurableAction {
                    operation: operation.to_owned(),
                    idempotency_key: key.to_owned(),
                    state,
                    target_type,
                    target_id,
                    request_payload,
                    failure,
                })
            })
            .transpose()
    }

    pub(crate) fn pending_external_actions(
        &self,
        operation: &str,
    ) -> Result<Vec<DurableAction>, CleanStoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT idempotency_key,target_type,target_id,request_payload,failure
                 FROM durable_actions WHERE operation=?1 AND state='pending'
                 ORDER BY created_at,idempotency_key LIMIT ?2",
            )
            .map_err(CleanStoreError::Database)?;
        let actions = statement
            .query_map(
                params![
                    operation,
                    i64::try_from(MAX_PENDING_ACTION_BATCH)
                        .map_err(|_| CleanStoreError::CorruptState)?
                ],
                |row| {
                    Ok(DurableAction {
                        operation: operation.to_owned(),
                        idempotency_key: row.get(0)?,
                        state: DurableActionState::Pending,
                        target_type: row.get(1)?,
                        target_id: row.get(2)?,
                        request_payload: row.get(3)?,
                        failure: row.get(4)?,
                    })
                },
            )
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        Ok(actions)
    }

    pub(crate) fn begin_external_action<M: Message>(
        &mut self,
        operation: &str,
        key: &str,
        request: &M,
        target_type: &str,
        target_id: &str,
    ) -> Result<DurableAction, CleanStoreError> {
        let digest = request_digest(operation, request);
        if let Some(existing) = action_row(&self.connection, operation, key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return Ok(existing.into_action(operation, key));
        }
        let now = unix_seconds()?;
        let request_payload = request.encode_to_vec();
        self.connection
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,
                    created_at,updated_at
                 ) VALUES (?1,?2,?3,?4,'pending',?5,?6,?7,?7)",
                params![
                    operation,
                    key,
                    digest,
                    request_payload,
                    target_type,
                    target_id,
                    now
                ],
            )
            .map_err(CleanStoreError::Database)?;
        Ok(DurableAction {
            operation: operation.to_owned(),
            idempotency_key: key.to_owned(),
            state: DurableActionState::Pending,
            target_type: Some(target_type.to_owned()),
            target_id: Some(target_id.to_owned()),
            request_payload,
            failure: None,
        })
    }

    fn record_completed_action<M: Message>(
        &mut self,
        operation: &str,
        key: &str,
        request: &M,
        target_type: &str,
        target_id: &str,
    ) -> Result<(), CleanStoreError> {
        let now = unix_seconds()?;
        self.connection
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,
                    created_at,updated_at
                 ) VALUES (?1,?2,?3,?4,'completed',?5,?6,?7,?7)",
                params![
                    operation,
                    key,
                    request_digest(operation, request),
                    request.encode_to_vec(),
                    target_type,
                    target_id,
                    now
                ],
            )
            .map_err(map_constraint)?;
        Ok(())
    }

    pub(crate) fn complete_external_action(
        &mut self,
        operation: &str,
        key: &str,
    ) -> Result<(), CleanStoreError> {
        let changed = self
            .connection
            .execute(
                "UPDATE durable_actions SET state='completed', failure=NULL, updated_at=?3
                 WHERE operation=?1 AND idempotency_key=?2 AND state='pending'",
                params![operation, key, unix_seconds()?],
            )
            .map_err(CleanStoreError::Database)?;
        if changed == 0 {
            let action = self.durable_action(operation, key)?;
            if action.is_some_and(|value| value.state == DurableActionState::Completed) {
                Ok(())
            } else {
                Err(CleanStoreError::InvalidTransition)
            }
        } else {
            Ok(())
        }
    }

    pub(crate) fn fail_external_action(
        &mut self,
        operation: &str,
        key: &str,
        failure: &str,
    ) -> Result<(), CleanStoreError> {
        let failure = truncate_utf8(failure, 512);
        let changed = self
            .connection
            .execute(
                "UPDATE durable_actions SET state='failed', failure=?3, updated_at=?4
                 WHERE operation=?1 AND idempotency_key=?2 AND state='pending'",
                params![operation, key, failure, unix_seconds()?],
            )
            .map_err(CleanStoreError::Database)?;
        if changed == 1 {
            return Ok(());
        }
        let action = self.durable_action(operation, key)?;
        if action.is_some_and(|value| {
            value.state == DurableActionState::Failed
                && value.failure.as_deref() == Some(failure.as_str())
        }) {
            Ok(())
        } else {
            Err(CleanStoreError::InvalidTransition)
        }
    }

    pub(crate) fn requeue_external_action(
        &mut self,
        operation: &str,
        key: &str,
    ) -> Result<(), CleanStoreError> {
        let changed = self
            .connection
            .execute(
                "UPDATE durable_actions SET state='pending', failure=NULL, updated_at=?3
                 WHERE operation=?1 AND idempotency_key=?2 AND state='failed'",
                params![operation, key, unix_seconds()?],
            )
            .map_err(CleanStoreError::Database)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(CleanStoreError::InvalidTransition)
        }
    }

    fn page_messages<M: Message + Default>(
        &mut self,
        method: &str,
        filter_hash: &str,
        page: Option<&v1::PageRequest>,
        fresh_items: impl FnOnce(&Connection) -> Result<Vec<Vec<u8>>, CleanStoreError>,
        service: &str,
    ) -> Result<Page<M>, CleanStoreError> {
        let size = page_size(page);
        let token = page.map_or("", |value| value.page_token.as_str());
        if token.is_empty() {
            let items = fresh_items(&self.connection)?;
            let cursor = self.current_cursor(service)?;
            return self.materialize_page(method, filter_hash, cursor, items, 0, size);
        }
        let page_token = decode_page_token(token)?;
        let row = self
            .connection
            .query_row(
                "SELECT method,filter_hash,cursor,items,token_secret,created_at
                 FROM page_snapshots WHERE token=?1",
                [&page_token.snapshot],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(CleanStoreError::Database)?
            .ok_or(CleanStoreError::InvalidPageToken)?;
        let now = unix_seconds()?;
        if row.0 != method
            || row.1 != filter_hash
            || now.saturating_sub(row.5) > PAGE_LIFETIME_SECONDS
            || page_token.proof != page_token_proof(&row.4, &page_token.snapshot, page_token.offset)
        {
            return Err(CleanStoreError::InvalidPageToken);
        }
        let encoded: Vec<String> =
            serde_json::from_str(&row.3).map_err(|_| CleanStoreError::CorruptState)?;
        let items = encoded
            .into_iter()
            .map(|item| {
                URL_SAFE_NO_PAD
                    .decode(item)
                    .map_err(|_| CleanStoreError::CorruptState)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.materialize_existing_page(
            &page_token.snapshot,
            &row.4,
            u64::try_from(row.2).map_err(|_| CleanStoreError::CorruptState)?,
            items,
            page_token.offset,
            size,
        )
    }

    fn materialize_page<M: Message + Default>(
        &mut self,
        method: &str,
        filter_hash: &str,
        cursor: u64,
        items: Vec<Vec<u8>>,
        offset: usize,
        size: usize,
    ) -> Result<Page<M>, CleanStoreError> {
        if items.len() > MAX_PAGE_SNAPSHOT_ITEMS
            || items
                .iter()
                .try_fold(0_usize, |total, item| {
                    total.checked_add(item.len()).ok_or(())
                })
                .map_or(true, |total| total > MAX_PAGE_SNAPSHOT_BYTES)
        {
            return Err(CleanStoreError::BoundExceeded);
        }
        let fits_one_page = items.len() <= size
            && items.iter().map(Vec::len).sum::<usize>() <= MAX_PAGE_RESPONSE_BYTES;
        if fits_one_page {
            let values = items
                .into_iter()
                .map(|value| decode(&value))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Page {
                items: values,
                next_page_token: String::new(),
                snapshot_cursor: cursor,
            });
        }
        self.prune_page_snapshots()?;
        let token = Uuid::now_v7().to_string();
        let secret = Uuid::now_v7().to_string();
        let encoded = items
            .iter()
            .map(|value| URL_SAFE_NO_PAD.encode(value))
            .collect::<Vec<_>>();
        let encoded = serde_json::to_string(&encoded).map_err(|_| CleanStoreError::CorruptState)?;
        if encoded.len() > MAX_PAGE_SNAPSHOT_BYTES.saturating_mul(2) {
            return Err(CleanStoreError::BoundExceeded);
        }
        self.connection
            .execute(
                "INSERT INTO page_snapshots(token,method,filter_hash,cursor,items,token_secret,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    token,
                    method,
                    filter_hash,
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                    encoded,
                    secret,
                    unix_seconds()?
                ],
            )
            .map_err(CleanStoreError::Database)?;
        self.materialize_existing_page(&token, &secret, cursor, items, offset, size)
    }

    fn materialize_existing_page<M: Message + Default>(
        &self,
        token: &str,
        secret: &str,
        cursor: u64,
        items: Vec<Vec<u8>>,
        offset: usize,
        size: usize,
    ) -> Result<Page<M>, CleanStoreError> {
        if offset > items.len() {
            return Err(CleanStoreError::InvalidPageToken);
        }
        let mut end = offset;
        let mut bytes = 0_usize;
        while end < items.len() && end.saturating_sub(offset) < size {
            let next = bytes
                .checked_add(items[end].len())
                .ok_or(CleanStoreError::BoundExceeded)?;
            if next > MAX_PAGE_RESPONSE_BYTES {
                break;
            }
            bytes = next;
            end = end.saturating_add(1);
        }
        if end == offset && offset < items.len() {
            return Err(CleanStoreError::BoundExceeded);
        }
        let values = items[offset..end]
            .iter()
            .map(|value| decode(value))
            .collect::<Result<Vec<_>, _>>()?;
        let next_page_token = if end < items.len() {
            encode_page_token(token, secret, end)?
        } else {
            String::new()
        };
        Ok(Page {
            items: values,
            next_page_token,
            snapshot_cursor: cursor,
        })
    }

    fn prune_page_snapshots(&mut self) -> Result<(), CleanStoreError> {
        let now = unix_seconds()?;
        self.connection
            .execute(
                "DELETE FROM page_snapshots WHERE created_at<?1",
                [now.saturating_sub(PAGE_LIFETIME_SECONDS)],
            )
            .map_err(CleanStoreError::Database)?;
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM page_snapshots", [], |row| row.get(0))
            .map_err(CleanStoreError::Database)?;
        let excess = usize::try_from(count)
            .unwrap_or(usize::MAX)
            .saturating_sub(MAX_PAGE_SNAPSHOTS.saturating_sub(1));
        if excess > 0 {
            self.connection
                .execute(
                    "DELETE FROM page_snapshots WHERE token IN (
                        SELECT token FROM page_snapshots ORDER BY created_at ASC LIMIT ?1
                     )",
                    [i64::try_from(excess).map_err(|_| CleanStoreError::CorruptState)?],
                )
                .map_err(CleanStoreError::Database)?;
        }
        Ok(())
    }

    pub(crate) fn create_repository_connection(
        &mut self,
        key: &str,
        request: &v1::CreateRepositoryConnectionRequest,
    ) -> Result<v1::RepositoryConnection, CleanStoreError> {
        let digest = request_digest("CreateRepositoryConnection", request);
        if let Some(existing) = action_row(&self.connection, "CreateRepositoryConnection", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            let id = existing.target_id.ok_or(CleanStoreError::CorruptState)?;
            return self.repository_connection(&id);
        }
        let provider = request
            .provider
            .clone()
            .ok_or(CleanStoreError::CorruptState)?;
        let provider_key = provider_key(&provider)?;
        let now = unix_seconds()?;
        let id = resource_id("repository");
        let mut resource = v1::RepositoryConnection {
            id: id.clone(),
            provider: Some(provider),
            display_name: request.display_name.clone(),
            default_branch: request.default_branch.clone(),
            status: v1::RepositoryConnectionStatus::Connecting as i32,
            failure: None,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('CreateRepositoryConnection',?1,?2,?3,'completed','repository_connection',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), id, now],
            )
            .map_err(map_constraint)?;
        let cursor = append_event(
            &transaction,
            REPOSITORY_SERVICE,
            "repository_connection",
            &id,
            1,
            Some(&id),
            None,
            &resource,
        )?;
        transaction
            .execute(
                "INSERT INTO repository_connections(id,status,provider_key,payload,resource_version,event_cursor)
                 VALUES (?1,?2,?3,?4,1,?5)",
                params![
                    id,
                    resource.status,
                    provider_key,
                    resource.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        // The payload is immutable for this transaction; cursor is an observation identity,
        // not a field in the public resource.
        resource.resource_version = 1;
        Ok(resource)
    }

    pub(crate) fn repository_connection(
        &self,
        id: &str,
    ) -> Result<v1::RepositoryConnection, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM repository_connections WHERE id=?1",
            id,
            "repository_connection",
        )
    }

    pub(crate) fn transition_repository_connection(
        &mut self,
        id: &str,
        expected_version: u64,
        state: v1::RepositoryConnectionStatus,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::RepositoryConnection, CleanStoreError> {
        let mut resource = self.repository_connection(id)?;
        if resource.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let current = v1::RepositoryConnectionStatus::try_from(resource.status)
            .map_err(|_| CleanStoreError::CorruptState)?;
        if !valid_repository_transition(current, state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        resource.status = state as i32;
        resource.failure = bounded_failure(failure);
        resource.updated_at = Some(timestamp(unix_seconds()?));
        resource.resource_version = resource.resource_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            REPOSITORY_SERVICE,
            "repository_connection",
            &resource.id,
            resource.resource_version,
            Some(&resource.id),
            None,
            &resource,
        )?;
        let changed = transaction.execute(
            "UPDATE repository_connections SET status=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
            params![resource.status, resource.encode_to_vec(),
                i64::try_from(resource.resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?, resource.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?],
        ).map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(resource)
    }

    pub(crate) fn list_repository_connections(
        &mut self,
        status: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::RepositoryConnection>, CleanStoreError> {
        let status_text = status.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[&status_text]);
        self.page_messages(
            "ListRepositoryConnections",
            &filter,
            page,
            |connection| query_payloads(connection, "repository_connections", "", status, None),
            REPOSITORY_SERVICE,
        )
    }

    pub(crate) fn observe_repository_snapshot(
        &mut self,
        repository_connection_id: &str,
        revision: &str,
    ) -> Result<v1::RepositorySnapshot, CleanStoreError> {
        self.repository_connection(repository_connection_id)?;
        if let Some(payload) = self
            .connection
            .query_row(
                "SELECT payload FROM repository_snapshots
                 WHERE repository_connection_id=?1 AND revision=?2",
                params![repository_connection_id, revision],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?
        {
            return decode(&payload);
        }
        let connection = self.repository_connection(repository_connection_id)?;
        let id = resource_id("snapshot");
        let snapshot = v1::RepositorySnapshot {
            id: id.clone(),
            repository_connection_id: repository_connection_id.to_owned(),
            revision: revision.to_owned(),
            default_branch: connection.default_branch,
            observed_at: Some(timestamp(unix_seconds()?)),
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            REPOSITORY_SERVICE,
            "repository_snapshot",
            &id,
            1,
            Some(repository_connection_id),
            Some(repository_connection_id),
            &snapshot,
        )?;
        transaction
            .execute(
                "INSERT INTO repository_snapshots(
                    id,repository_connection_id,revision,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,1,?5)",
                params![
                    id,
                    repository_connection_id,
                    revision,
                    snapshot.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(snapshot)
    }

    pub(crate) fn repository_snapshot(
        &self,
        id: &str,
    ) -> Result<v1::RepositorySnapshot, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM repository_snapshots WHERE id=?1",
            id,
            "repository_snapshot",
        )
    }

    pub(crate) fn repository_snapshot_for_revision(
        &self,
        repository_connection_id: &str,
        revision: &str,
    ) -> Result<v1::RepositorySnapshot, CleanStoreError> {
        let payload = self
            .connection
            .query_row(
                "SELECT payload FROM repository_snapshots
                 WHERE repository_connection_id=?1 AND revision=?2",
                params![repository_connection_id, revision],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?
            .ok_or_else(|| CleanStoreError::NotFound {
                resource_type: "repository_snapshot",
                resource_id: revision.to_owned(),
            })?;
        decode(&payload)
    }

    pub(crate) fn list_repository_snapshots(
        &mut self,
        repository_connection_id: &str,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::RepositorySnapshot>, CleanStoreError> {
        self.repository_connection(repository_connection_id)?;
        let filter = encode_filter(&[repository_connection_id]);
        self.page_messages(
            "ListRepositorySnapshots",
            &filter,
            page,
            |connection| {
                query_payloads(
                    connection,
                    "repository_snapshots",
                    "repository_connection_id",
                    None,
                    Some(repository_connection_id),
                )
            },
            REPOSITORY_SERVICE,
        )
    }

    pub(crate) fn create_runtime_profile(
        &mut self,
        key: &str,
        request: &v1::CreateRuntimeProfileRequest,
    ) -> Result<(v1::RuntimeProfile, v1::RuntimeProfileRevision), CleanStoreError> {
        let digest = request_digest("CreateRuntimeProfile", request);
        if let Some(existing) = action_row(&self.connection, "CreateRuntimeProfile", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            let profile_id = existing.target_id.ok_or(CleanStoreError::CorruptState)?;
            let profile = self.runtime_profile(&profile_id)?;
            let revision: v1::RuntimeProfileRevision = get_payload(
                &self.connection,
                "SELECT payload FROM runtime_profile_revisions
                 WHERE runtime_profile_id=?1 AND revision_number=1",
                &profile_id,
                "runtime_profile_revision",
            )?;
            return Ok((profile, revision));
        }
        let repository = self.repository_connection(&request.repository_connection_id)?;
        let snapshot = self.repository_snapshot(&request.repository_snapshot_id)?;
        if snapshot.repository_connection_id != repository.id {
            return Err(CleanStoreError::ParentMismatch);
        }
        let configuration = request
            .configuration
            .as_ref()
            .and_then(|value| value.configuration.clone())
            .ok_or(CleanStoreError::CorruptState)?;
        let (kind, revision_configuration) = match configuration {
            v1::runtime_profile_configuration::Configuration::AyniManaged(value) => (
                v1::RuntimeProfileKind::AyniManaged,
                v1::runtime_profile_revision::Configuration::AyniManaged(value),
            ),
            v1::runtime_profile_configuration::Configuration::CustomRuntime(value) => (
                v1::RuntimeProfileKind::CustomRuntime,
                v1::runtime_profile_revision::Configuration::CustomRuntime(value),
            ),
        };
        let now = unix_seconds()?;
        let profile_id = resource_id("profile");
        let revision_id = resource_id("profile-revision");
        let operational = kind == v1::RuntimeProfileKind::AyniManaged;
        let profile = v1::RuntimeProfile {
            id: profile_id.clone(),
            repository_connection_id: repository.id.clone(),
            kind: kind as i32,
            support: if operational {
                v1::RuntimeProfileSupport::Operational as i32
            } else {
                v1::RuntimeProfileSupport::Unsupported as i32
            },
            state: if operational {
                v1::RuntimeProfileState::SetupRequired as i32
            } else {
                v1::RuntimeProfileState::Failed as i32
            },
            active_revision_id: Some(revision_id.clone()),
            active_runtime_artifact_id: None,
            failure: None,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let revision = v1::RuntimeProfileRevision {
            id: revision_id.clone(),
            runtime_profile_id: profile_id.clone(),
            repository_snapshot_id: snapshot.id,
            repository_revision: snapshot.revision,
            revision_number: 1,
            configuration: Some(revision_configuration),
            created_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('CreateRuntimeProfile',?1,?2,?3,'completed','runtime_profile',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), profile_id, now],
            )
            .map_err(map_constraint)?;
        let profile_cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_profile",
            &profile_id,
            1,
            Some(&repository.id),
            Some(&profile_id),
            &profile,
        )?;
        transaction
            .execute(
                "INSERT INTO runtime_profiles(
                    id,repository_connection_id,kind,state,active_revision_id,
                    active_runtime_artifact_id,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,NULL,?6,1,?7)",
                params![
                    profile_id,
                    repository.id,
                    profile.kind,
                    profile.state,
                    revision_id,
                    profile.encode_to_vec(),
                    i64::try_from(profile_cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        let revision_cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_profile_revision",
            &revision_id,
            1,
            Some(&profile.repository_connection_id),
            Some(&profile_id),
            &revision,
        )?;
        transaction
            .execute(
                "INSERT INTO runtime_profile_revisions(
                    id,runtime_profile_id,repository_snapshot_id,revision_number,payload,
                    resource_version,event_cursor
                 ) VALUES (?1,?2,?3,1,?4,1,?5)",
                params![
                    revision_id,
                    profile_id,
                    revision.repository_snapshot_id,
                    revision.encode_to_vec(),
                    i64::try_from(revision_cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok((profile, revision))
    }

    pub(crate) fn runtime_profile(&self, id: &str) -> Result<v1::RuntimeProfile, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM runtime_profiles WHERE id=?1",
            id,
            "runtime_profile",
        )
    }

    pub(crate) fn transition_runtime_profile(
        &mut self,
        id: &str,
        expected_version: u64,
        state: v1::RuntimeProfileState,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::RuntimeProfile, CleanStoreError> {
        let mut resource = self.runtime_profile(id)?;
        if resource.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let current = v1::RuntimeProfileState::try_from(resource.state)
            .map_err(|_| CleanStoreError::CorruptState)?;
        if !valid_profile_transition(current, state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        resource.state = state as i32;
        resource.failure = bounded_failure(failure);
        resource.updated_at = Some(timestamp(unix_seconds()?));
        resource.resource_version = resource.resource_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_profile",
            &resource.id,
            resource.resource_version,
            Some(&resource.repository_connection_id),
            Some(&resource.id),
            &resource,
        )?;
        let changed = transaction.execute(
            "UPDATE runtime_profiles SET state=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
            params![resource.state, resource.encode_to_vec(),
                i64::try_from(resource.resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?, resource.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?],
        ).map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(resource)
    }

    pub(crate) fn list_runtime_profiles(
        &mut self,
        repository_connection_id: Option<&str>,
        kind: Option<i32>,
        state: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::RuntimeProfile>, CleanStoreError> {
        let kind_text = kind.map_or_else(String::new, |value| value.to_string());
        let state_text = state.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[
            repository_connection_id.unwrap_or(""),
            &kind_text,
            &state_text,
        ]);
        self.page_messages(
            "ListRuntimeProfiles",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM runtime_profiles
                         WHERE (?1 IS NULL OR repository_connection_id=?1)
                           AND (?2 IS NULL OR kind=?2)
                           AND (?3 IS NULL OR state=?3)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![repository_connection_id, kind, state])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            PROFILE_SERVICE,
        )
    }

    pub(crate) fn runtime_profile_revision(
        &self,
        id: &str,
    ) -> Result<v1::RuntimeProfileRevision, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM runtime_profile_revisions WHERE id=?1",
            id,
            "runtime_profile_revision",
        )
    }

    pub(crate) fn list_runtime_profile_revisions(
        &mut self,
        profile_id: &str,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::RuntimeProfileRevision>, CleanStoreError> {
        self.runtime_profile(profile_id)?;
        let filter = encode_filter(&[profile_id]);
        self.page_messages(
            "ListRuntimeProfileRevisions",
            &filter,
            page,
            |connection| {
                query_payloads(
                    connection,
                    "runtime_profile_revisions",
                    "runtime_profile_id",
                    None,
                    Some(profile_id),
                )
            },
            PROFILE_SERVICE,
        )
    }

    pub(crate) fn record_runtime_profile_revision(
        &mut self,
        profile_id: &str,
        repository_snapshot_id: &str,
        configuration: v1::runtime_profile_revision::Configuration,
    ) -> Result<v1::RuntimeProfileRevision, CleanStoreError> {
        let mut profile = self.runtime_profile(profile_id)?;
        let snapshot = self.repository_snapshot(repository_snapshot_id)?;
        if snapshot.repository_connection_id != profile.repository_connection_id {
            return Err(CleanStoreError::ParentMismatch);
        }
        let expected_kind = match &configuration {
            v1::runtime_profile_revision::Configuration::AyniManaged(_) => {
                v1::RuntimeProfileKind::AyniManaged as i32
            }
            v1::runtime_profile_revision::Configuration::CustomRuntime(_) => {
                v1::RuntimeProfileKind::CustomRuntime as i32
            }
        };
        if profile.kind != expected_kind {
            return Err(CleanStoreError::ParentMismatch);
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT payload FROM runtime_profile_revisions
                 WHERE runtime_profile_id=?1 ORDER BY revision_number DESC",
            )
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement
            .query([profile_id])
            .map_err(CleanStoreError::Database)?;
        let payloads = collect_bounded_payloads(&mut rows)?;
        drop(rows);
        drop(statement);
        let revisions = payloads
            .iter()
            .map(|payload| decode::<v1::RuntimeProfileRevision>(payload))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(existing) = revisions.iter().find(|existing| {
            existing.repository_snapshot_id == repository_snapshot_id
                && existing.configuration.as_ref() == Some(&configuration)
        }) {
            return Ok(existing.clone());
        }
        let revision_number = revisions
            .first()
            .map_or(1, |value| value.revision_number.saturating_add(1));
        let id = resource_id("profile-revision");
        let revision = v1::RuntimeProfileRevision {
            id: id.clone(),
            runtime_profile_id: profile.id.clone(),
            repository_snapshot_id: snapshot.id,
            repository_revision: snapshot.revision,
            revision_number,
            configuration: Some(configuration),
            created_at: Some(timestamp(unix_seconds()?)),
            resource_version: 1,
        };
        let previous_version = profile.resource_version;
        profile.active_revision_id = Some(id.clone());
        profile.active_runtime_artifact_id = None;
        profile.state = v1::RuntimeProfileState::Updating as i32;
        profile.failure = None;
        profile.updated_at = Some(timestamp(unix_seconds()?));
        profile.resource_version = previous_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let revision_cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_profile_revision",
            &id,
            1,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &revision,
        )?;
        transaction
            .execute(
                "INSERT INTO runtime_profile_revisions(
                id,runtime_profile_id,repository_snapshot_id,revision_number,payload,
                resource_version,event_cursor
             ) VALUES (?1,?2,?3,?4,?5,1,?6)",
                params![
                    id,
                    profile.id,
                    revision.repository_snapshot_id,
                    i64::try_from(revision_number).map_err(|_| CleanStoreError::CorruptState)?,
                    revision.encode_to_vec(),
                    i64::try_from(revision_cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        let profile_cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_profile",
            &profile.id,
            profile.resource_version,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &profile,
        )?;
        let changed = transaction
            .execute(
                "UPDATE runtime_profiles SET state=?1,active_revision_id=?2,
                active_runtime_artifact_id=NULL,payload=?3,resource_version=?4,event_cursor=?5
             WHERE id=?6 AND resource_version=?7",
                params![
                    profile.state,
                    profile.active_revision_id,
                    profile.encode_to_vec(),
                    i64::try_from(profile.resource_version)
                        .map_err(|_| CleanStoreError::CorruptState)?,
                    i64::try_from(profile_cursor).map_err(|_| CleanStoreError::CorruptState)?,
                    profile.id,
                    i64::try_from(previous_version).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(revision)
    }

    pub(crate) fn record_runtime_artifact(
        &mut self,
        profile_id: &str,
        mut artifact: v1::RuntimeArtifact,
    ) -> Result<v1::RuntimeArtifact, CleanStoreError> {
        let mut profile = self.runtime_profile(profile_id)?;
        let revision = self.runtime_profile_revision(&artifact.runtime_profile_revision_id)?;
        if revision.runtime_profile_id != profile.id {
            return Err(CleanStoreError::ParentMismatch);
        }
        artifact.failure = bounded_failure(artifact.failure);
        if !valid_artifact_report(&artifact) {
            return Err(CleanStoreError::BoundExceeded);
        }
        let platform = artifact
            .platform
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        let platform_key = format!(
            "{}/{}/{}",
            platform.operating_system,
            platform.architecture,
            platform.variant.as_deref().unwrap_or("")
        );
        let existing: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT payload FROM runtime_artifacts
                 WHERE runtime_profile_revision_id=?1 AND resolved_digest=?2 AND platform_key=?3",
                params![
                    artifact.runtime_profile_revision_id,
                    artifact.resolved_digest,
                    platform_key
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        if let Some(payload) = existing {
            let existing = decode::<v1::RuntimeArtifact>(&payload)?;
            return if same_artifact_report(&existing, &artifact) {
                Ok(existing)
            } else {
                Err(CleanStoreError::IdempotencyConflict)
            };
        }
        if artifact.id.is_empty() {
            artifact.id = resource_id("artifact");
        }
        artifact.resource_version = 1;
        if artifact.created_at.is_none() {
            artifact.created_at = Some(timestamp(unix_seconds()?));
        }
        let verified = artifact.status == v1::RuntimeArtifactStatus::Verified as i32;
        if verified && artifact.verified_at.is_none() {
            artifact.verified_at = Some(timestamp(unix_seconds()?));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_artifact",
            &artifact.id,
            1,
            Some(&profile.repository_connection_id),
            Some(profile_id),
            &artifact,
        )?;
        transaction
            .execute(
                "INSERT INTO runtime_artifacts(
                    id,runtime_profile_id,runtime_profile_revision_id,status,resolved_digest,
                    platform_key,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8)",
                params![
                    artifact.id,
                    profile_id,
                    artifact.runtime_profile_revision_id,
                    artifact.status,
                    artifact.resolved_digest,
                    platform_key,
                    artifact.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        if verified
            && profile.active_revision_id.as_deref() == Some(revision.id.as_str())
            && profile.active_runtime_artifact_id.is_none()
        {
            profile.active_runtime_artifact_id = Some(artifact.id.clone());
            profile.active_revision_id = Some(revision.id);
            profile.state = v1::RuntimeProfileState::Ready as i32;
            profile.failure = None;
            profile.updated_at = Some(timestamp(unix_seconds()?));
            profile.resource_version = profile.resource_version.saturating_add(1);
            let profile_cursor = append_event(
                &transaction,
                PROFILE_SERVICE,
                "runtime_profile",
                &profile.id,
                profile.resource_version,
                Some(&profile.repository_connection_id),
                Some(&profile.id),
                &profile,
            )?;
            let changed = transaction
                .execute(
                    "UPDATE runtime_profiles SET state=?1,active_revision_id=?2,
                        active_runtime_artifact_id=?3,payload=?4,resource_version=?5,event_cursor=?6
                     WHERE id=?7 AND resource_version=?8",
                    params![
                        profile.state,
                        profile.active_revision_id,
                        profile.active_runtime_artifact_id,
                        profile.encode_to_vec(),
                        i64::try_from(profile.resource_version)
                            .map_err(|_| CleanStoreError::CorruptState)?,
                        i64::try_from(profile_cursor).map_err(|_| CleanStoreError::CorruptState)?,
                        profile.id,
                        i64::try_from(profile.resource_version.saturating_sub(1))
                            .map_err(|_| CleanStoreError::CorruptState)?
                    ],
                )
                .map_err(CleanStoreError::Database)?;
            if changed != 1 {
                return Err(CleanStoreError::StaleVersion);
            }
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(artifact)
    }

    pub(crate) fn runtime_artifact(
        &self,
        id: &str,
    ) -> Result<v1::RuntimeArtifact, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM runtime_artifacts WHERE id=?1",
            id,
            "runtime_artifact",
        )
    }

    pub(crate) fn transition_runtime_artifact(
        &mut self,
        id: &str,
        expected_version: u64,
        state: v1::RuntimeArtifactStatus,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::RuntimeArtifact, CleanStoreError> {
        let mut resource = self.runtime_artifact(id)?;
        if resource.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let current = v1::RuntimeArtifactStatus::try_from(resource.status)
            .map_err(|_| CleanStoreError::CorruptState)?;
        if !valid_artifact_transition(current, state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        let profile_id: String = self
            .connection
            .query_row(
                "SELECT runtime_profile_id FROM runtime_artifacts WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        let mut profile = self.runtime_profile(&profile_id)?;
        resource.status = state as i32;
        resource.failure = bounded_failure(failure);
        if state == v1::RuntimeArtifactStatus::Verified && resource.verified_at.is_none() {
            resource.verified_at = Some(timestamp(unix_seconds()?));
        }
        resource.resource_version = resource.resource_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_artifact",
            &resource.id,
            resource.resource_version,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &resource,
        )?;
        let changed = transaction.execute(
            "UPDATE runtime_artifacts SET status=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
            params![resource.status, resource.encode_to_vec(),
                i64::try_from(resource.resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?, resource.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?],
        ).map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        let activates_current = state == v1::RuntimeArtifactStatus::Verified
            && profile.active_revision_id.as_deref()
                == Some(resource.runtime_profile_revision_id.as_str())
            && profile.active_runtime_artifact_id.is_none();
        let invalidates_current = state == v1::RuntimeArtifactStatus::Unavailable
            && profile.active_runtime_artifact_id.as_deref() == Some(resource.id.as_str());
        if activates_current || invalidates_current {
            let previous_profile_version = profile.resource_version;
            if activates_current {
                profile.active_runtime_artifact_id = Some(resource.id.clone());
                profile.state = v1::RuntimeProfileState::Ready as i32;
                profile.failure = None;
            } else {
                profile.active_runtime_artifact_id = None;
                profile.state = v1::RuntimeProfileState::Degraded as i32;
                profile.failure.clone_from(&resource.failure);
            }
            profile.updated_at = Some(timestamp(unix_seconds()?));
            profile.resource_version = previous_profile_version.saturating_add(1);
            let profile_cursor = append_event(
                &transaction,
                PROFILE_SERVICE,
                "runtime_profile",
                &profile.id,
                profile.resource_version,
                Some(&profile.repository_connection_id),
                Some(&profile.id),
                &profile,
            )?;
            let profile_changed = transaction
                .execute(
                    "UPDATE runtime_profiles SET state=?1,active_runtime_artifact_id=?2,
                        payload=?3,resource_version=?4,event_cursor=?5
                     WHERE id=?6 AND resource_version=?7",
                    params![
                        profile.state,
                        profile.active_runtime_artifact_id,
                        profile.encode_to_vec(),
                        i64::try_from(profile.resource_version)
                            .map_err(|_| CleanStoreError::CorruptState)?,
                        i64::try_from(profile_cursor).map_err(|_| CleanStoreError::CorruptState)?,
                        profile.id,
                        i64::try_from(previous_profile_version)
                            .map_err(|_| CleanStoreError::CorruptState)?
                    ],
                )
                .map_err(CleanStoreError::Database)?;
            if profile_changed != 1 {
                return Err(CleanStoreError::StaleVersion);
            }
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(resource)
    }

    pub(crate) fn list_runtime_artifacts(
        &mut self,
        profile_id: &str,
        revision_id: Option<&str>,
        status: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::RuntimeArtifact>, CleanStoreError> {
        self.runtime_profile(profile_id)?;
        let status_text = status.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[profile_id, revision_id.unwrap_or(""), &status_text]);
        self.page_messages(
            "ListRuntimeArtifacts",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM runtime_artifacts
                         WHERE runtime_profile_id=?1
                           AND (?2 IS NULL OR runtime_profile_revision_id=?2)
                           AND (?3 IS NULL OR status=?3)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![profile_id, revision_id, status])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            PROFILE_SERVICE,
        )
    }

    pub(crate) fn install_repository_automation(
        &mut self,
        key: &str,
        request: &v1::InstallRepositoryAutomationRequest,
    ) -> Result<v1::AutomationInstallation, CleanStoreError> {
        let digest = request_digest("InstallRepositoryAutomation", request);
        if let Some(existing) = action_row(&self.connection, "InstallRepositoryAutomation", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return self.automation_installation(
                &existing.target_id.ok_or(CleanStoreError::CorruptState)?,
            );
        }
        let profile = self.runtime_profile(&request.runtime_profile_id)?;
        let repository = self.repository_connection(&profile.repository_connection_id)?;
        let github = repository
            .provider
            .and_then(|provider| provider.provider)
            .map(|provider| match provider {
                v1::repository_provider::Provider::Github(value) => value,
            })
            .ok_or(CleanStoreError::CorruptState)?;
        let revision_id = profile
            .active_revision_id
            .ok_or(CleanStoreError::CorruptState)?;
        let revision = self.runtime_profile_revision(&revision_id)?;
        let version = match revision.configuration {
            Some(v1::runtime_profile_revision::Configuration::AyniManaged(value)) => {
                value.workflow_template_version
            }
            _ => return Err(CleanStoreError::InvalidTransition),
        };
        let now = unix_seconds()?;
        let id = resource_id("automation");
        let installation = v1::AutomationInstallation {
            id: id.clone(),
            runtime_profile_id: profile.id.clone(),
            version: version.clone(),
            state: v1::AutomationInstallationState::SetupRequired as i32,
            provider: Some(v1::automation_installation::Provider::Github(
                v1::GithubAutomationInstallation {
                    installation_id: github.installation_id,
                    setup_pull_request_number: None,
                    environment_workflow_path: String::new(),
                    assessment_workflow_path: String::new(),
                },
            )),
            failure: None,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('InstallRepositoryAutomation',?1,?2,?3,'pending','automation_installation',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), id, now],
            )
            .map_err(map_constraint)?;
        let cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "automation_installation",
            &id,
            1,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &installation,
        )?;
        transaction
            .execute(
                "INSERT INTO automation_installations(
                    id,runtime_profile_id,state,version,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,1,?6)",
                params![
                    id,
                    profile.id,
                    installation.state,
                    version,
                    installation.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(installation)
    }

    pub(crate) fn automation_installation(
        &self,
        id: &str,
    ) -> Result<v1::AutomationInstallation, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM automation_installations WHERE id=?1",
            id,
            "automation_installation",
        )
    }

    pub(crate) fn transition_automation_installation(
        &mut self,
        id: &str,
        expected_version: u64,
        state: v1::AutomationInstallationState,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::AutomationInstallation, CleanStoreError> {
        let mut resource = self.automation_installation(id)?;
        if resource.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let current = v1::AutomationInstallationState::try_from(resource.state)
            .map_err(|_| CleanStoreError::CorruptState)?;
        if !valid_automation_transition(current, state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        let profile = self.runtime_profile(&resource.runtime_profile_id)?;
        resource.state = state as i32;
        resource.failure = bounded_failure(failure);
        resource.updated_at = Some(timestamp(unix_seconds()?));
        resource.resource_version = resource.resource_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "automation_installation",
            &resource.id,
            resource.resource_version,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &resource,
        )?;
        let changed = transaction.execute(
            "UPDATE automation_installations SET state=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
            params![resource.state, resource.encode_to_vec(),
                i64::try_from(resource.resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?, resource.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?],
        ).map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(resource)
    }

    pub(crate) fn list_automation_installations(
        &mut self,
        profile_id: &str,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::AutomationInstallation>, CleanStoreError> {
        self.runtime_profile(profile_id)?;
        let filter = encode_filter(&[profile_id]);
        self.page_messages(
            "ListAutomationInstallations",
            &filter,
            page,
            |connection| {
                query_payloads(
                    connection,
                    "automation_installations",
                    "runtime_profile_id",
                    None,
                    Some(profile_id),
                )
            },
            PROFILE_SERVICE,
        )
    }

    pub(crate) fn reconcile_runtime_profile(
        &mut self,
        key: &str,
        request: &v1::ReconcileRuntimeProfileRequest,
    ) -> Result<v1::RuntimeProfile, CleanStoreError> {
        let profile = self.runtime_profile(&request.runtime_profile_id)?;
        let _action = self.begin_external_action(
            "ReconcileRuntimeProfile",
            key,
            request,
            "runtime_profile",
            &profile.id,
        )?;
        Ok(profile)
    }

    pub(crate) fn create_development_task(
        &mut self,
        key: &str,
        request: &v1::CreateDevelopmentTaskRequest,
    ) -> Result<(v1::DevelopmentTask, v1::DevelopmentAttempt), CleanStoreError> {
        let digest = request_digest("CreateDevelopmentTask", request);
        if let Some(existing) = action_row(&self.connection, "CreateDevelopmentTask", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            let task =
                self.development_task(&existing.target_id.ok_or(CleanStoreError::CorruptState)?)?;
            let initial_attempt: v1::DevelopmentAttempt = get_payload(
                &self.connection,
                "SELECT payload FROM development_attempts
                 WHERE development_task_id=?1 AND attempt_number=1",
                &task.id,
                "development_attempt",
            )?;
            return Ok((task, initial_attempt));
        }
        let requested = request
            .request
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        let repository = self.repository_connection(&requested.repository_connection_id)?;
        let repository_snapshot =
            self.repository_snapshot_for_revision(&repository.id, &requested.repository_revision)?;
        let profile = self.runtime_profile(&requested.runtime_profile_id)?;
        if profile.repository_connection_id != repository.id {
            return Err(CleanStoreError::ParentMismatch);
        }
        if profile.state != v1::RuntimeProfileState::Ready as i32
            || profile.support != v1::RuntimeProfileSupport::Operational as i32
        {
            return Err(CleanStoreError::ProfileNotReady);
        }
        let revision_id = profile
            .active_revision_id
            .clone()
            .ok_or(CleanStoreError::ProfileNotReady)?;
        let artifact_id = profile
            .active_runtime_artifact_id
            .clone()
            .ok_or(CleanStoreError::ProfileNotReady)?;
        let revision = self.runtime_profile_revision(&revision_id)?;
        let artifact = self.runtime_artifact(&artifact_id)?;
        if artifact.runtime_profile_revision_id != revision.id
            || artifact.status != v1::RuntimeArtifactStatus::Verified as i32
        {
            return Err(CleanStoreError::ProfileNotReady);
        }
        let now_timestamp = unix_timestamp()?;
        let now = now_timestamp.seconds;
        let deadline_at = requested
            .deadline
            .as_ref()
            .map(|duration| add_duration(&now_timestamp, duration))
            .transpose()?;
        let snapshot = v1::DevelopmentSnapshot {
            repository: Some(v1::FrozenRepository {
                repository_connection_id: repository.id.clone(),
                repository_snapshot_id: repository_snapshot.id,
                revision: repository_snapshot.revision,
            }),
            runtime: Some(v1::FrozenRuntime {
                runtime_profile_id: profile.id.clone(),
                runtime_profile_revision_id: revision.id,
                profile_kind: profile.kind,
                runtime_artifact_id: artifact.id,
                oci_repository: artifact.oci_repository,
                resolved_digest: artifact.resolved_digest,
                platform: artifact.platform,
            }),
            agent: requested.agent.clone(),
            request: requested.request.clone(),
            acceptance_criteria: requested.acceptance_criteria.clone(),
            change_constraints: requested.change_constraints.clone(),
            deadline_at,
        };
        let task_id = resource_id("task");
        let attempt_id = resource_id("attempt");
        let task = v1::DevelopmentTask {
            id: task_id.clone(),
            snapshot: Some(snapshot.clone()),
            state: v1::DevelopmentTaskState::Queued as i32,
            active_attempt_id: Some(attempt_id.clone()),
            attempt_count: 1,
            failure: None,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let attempt = v1::DevelopmentAttempt {
            id: attempt_id.clone(),
            development_task_id: task_id.clone(),
            attempt_number: 1,
            snapshot: Some(snapshot),
            state: v1::DevelopmentAttemptState::Queued as i32,
            requested_input: None,
            failure: None,
            model_usage: Some(v1::AttemptModelUsage {
                collection_status: v1::ModelUsageCollectionStatus::Unavailable as i32,
                ..v1::AttemptModelUsage::default()
            }),
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            completed_at: None,
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('CreateDevelopmentTask',?1,?2,?3,'completed','development_task',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), task_id, now],
            )
            .map_err(map_constraint)?;
        let task_cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "development_task",
            &task_id,
            1,
            Some(&repository.id),
            Some(&task_id),
            &task,
        )?;
        transaction
            .execute(
                "INSERT INTO development_tasks(
                    id,repository_connection_id,runtime_profile_id,state,active_attempt_id,
                    deadline_at_seconds,deadline_at_nanos,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1,?9)",
                params![
                    task_id,
                    repository.id,
                    profile.id,
                    task.state,
                    attempt_id,
                    task.snapshot
                        .as_ref()
                        .and_then(|value| value.deadline_at.as_ref())
                        .map(|value| value.seconds),
                    task.snapshot
                        .as_ref()
                        .and_then(|value| value.deadline_at.as_ref())
                        .map(|value| value.nanos),
                    task.encode_to_vec(),
                    i64::try_from(task_cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        let attempt_cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "development_attempt",
            &attempt_id,
            1,
            Some(&repository.id),
            Some(&task_id),
            &attempt,
        )?;
        transaction
            .execute(
                "INSERT INTO development_attempts(
                    id,development_task_id,attempt_number,state,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,1,?3,?4,1,?5)",
                params![
                    attempt_id,
                    task_id,
                    attempt.state,
                    attempt.encode_to_vec(),
                    i64::try_from(attempt_cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok((task, attempt))
    }

    pub(crate) fn development_task(
        &self,
        id: &str,
    ) -> Result<v1::DevelopmentTask, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM development_tasks WHERE id=?1",
            id,
            "development_task",
        )
    }

    pub(crate) fn list_development_tasks(
        &mut self,
        repository_connection_id: Option<&str>,
        runtime_profile_id: Option<&str>,
        state: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::DevelopmentTask>, CleanStoreError> {
        let state_text = state.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[
            repository_connection_id.unwrap_or(""),
            runtime_profile_id.unwrap_or(""),
            &state_text,
        ]);
        self.page_messages(
            "ListDevelopmentTasks",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM development_tasks
                         WHERE (?1 IS NULL OR repository_connection_id=?1)
                           AND (?2 IS NULL OR runtime_profile_id=?2)
                           AND (?3 IS NULL OR state=?3)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![repository_connection_id, runtime_profile_id, state])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            DEVELOPMENT_SERVICE,
        )
    }

    pub(crate) fn development_attempt(
        &self,
        id: &str,
    ) -> Result<v1::DevelopmentAttempt, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM development_attempts WHERE id=?1",
            id,
            "development_attempt",
        )
    }

    pub(crate) fn list_development_attempts(
        &mut self,
        task_id: &str,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::DevelopmentAttempt>, CleanStoreError> {
        self.development_task(task_id)?;
        let filter = encode_filter(&[task_id]);
        self.page_messages(
            "ListDevelopmentAttempts",
            &filter,
            page,
            |connection| {
                query_payloads(
                    connection,
                    "development_attempts",
                    "development_task_id",
                    None,
                    Some(task_id),
                )
            },
            DEVELOPMENT_SERVICE,
        )
    }

    pub(crate) fn cancel_development_task(
        &mut self,
        key: &str,
        request: &v1::CancelDevelopmentTaskRequest,
    ) -> Result<v1::DevelopmentTask, CleanStoreError> {
        let digest = request_digest("CancelDevelopmentTask", request);
        if let Some(existing) = action_row(&self.connection, "CancelDevelopmentTask", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return self
                .development_task(&existing.target_id.ok_or(CleanStoreError::CorruptState)?);
        }
        let mut task = self.development_task(&request.development_task_id)?;
        if is_terminal_task(task.state) {
            if task.state == v1::DevelopmentTaskState::Cancelled as i32 {
                self.record_completed_action(
                    "CancelDevelopmentTask",
                    key,
                    request,
                    "development_task",
                    &task.id,
                )?;
                return Ok(task);
            }
            return Err(CleanStoreError::TaskNotActionable);
        }
        let now = unix_seconds()?;
        task.state = v1::DevelopmentTaskState::Cancelled as i32;
        task.failure = None;
        task.updated_at = Some(timestamp(now));
        let previous_version = task.resource_version;
        task.resource_version = previous_version.saturating_add(1);
        let mut attempt = task
            .active_attempt_id
            .as_deref()
            .map(|id| self.development_attempt(id))
            .transpose()?;
        if let Some(value) = attempt.as_mut()
            && !is_terminal_attempt(value.state)
        {
            value.state = v1::DevelopmentAttemptState::Cancelled as i32;
            value.updated_at = Some(timestamp(now));
            value.completed_at = Some(timestamp(now));
            value.failure = None;
            value.resource_version = value.resource_version.saturating_add(1);
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('CancelDevelopmentTask',?1,?2,?3,'completed','development_task',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), task.id, now],
            )
            .map_err(map_constraint)?;
        persist_task_update(&transaction, &task, previous_version)?;
        if let Some(value) = attempt {
            persist_attempt_update(
                &transaction,
                &value,
                value.resource_version.saturating_sub(1),
                &task,
            )?;
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(task)
    }

    pub(crate) fn retry_development_task(
        &mut self,
        key: &str,
        request: &v1::RetryDevelopmentTaskRequest,
    ) -> Result<v1::DevelopmentAttempt, CleanStoreError> {
        let digest = request_digest("RetryDevelopmentTask", request);
        if let Some(existing) = action_row(&self.connection, "RetryDevelopmentTask", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return self
                .development_attempt(&existing.target_id.ok_or(CleanStoreError::CorruptState)?);
        }
        let mut task = self.development_task(&request.development_task_id)?;
        if !matches!(
            v1::DevelopmentTaskState::try_from(task.state).ok(),
            Some(v1::DevelopmentTaskState::Failed | v1::DevelopmentTaskState::DeadlineExceeded)
        ) || task.attempt_count >= self.max_attempts
        {
            return Err(CleanStoreError::TaskNotActionable);
        }
        let now_timestamp = unix_timestamp()?;
        let now = now_timestamp.seconds;
        if task
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.deadline_at.as_ref())
            .is_some_and(|deadline| timestamp_at_or_before(deadline, &now_timestamp))
        {
            return Err(CleanStoreError::DeadlineExpired);
        }
        let attempt_number = task.attempt_count.saturating_add(1);
        let attempt_id = resource_id("attempt");
        let attempt = v1::DevelopmentAttempt {
            id: attempt_id.clone(),
            development_task_id: task.id.clone(),
            attempt_number,
            snapshot: task.snapshot.clone(),
            state: v1::DevelopmentAttemptState::Queued as i32,
            requested_input: None,
            failure: None,
            model_usage: Some(v1::AttemptModelUsage {
                collection_status: v1::ModelUsageCollectionStatus::Unavailable as i32,
                ..v1::AttemptModelUsage::default()
            }),
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            completed_at: None,
            resource_version: 1,
        };
        let previous_version = task.resource_version;
        task.state = v1::DevelopmentTaskState::Queued as i32;
        task.active_attempt_id = Some(attempt_id.clone());
        task.attempt_count = attempt_number;
        task.failure = None;
        task.updated_at = Some(timestamp(now));
        task.resource_version = previous_version.saturating_add(1);
        let repository_id = task
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.repository.as_ref())
            .map(|repository| repository.repository_connection_id.clone())
            .ok_or(CleanStoreError::CorruptState)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('RetryDevelopmentTask',?1,?2,?3,'completed','development_attempt',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), attempt_id, now],
            )
            .map_err(map_constraint)?;
        persist_task_update(&transaction, &task, previous_version)?;
        let cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "development_attempt",
            &attempt.id,
            1,
            Some(&repository_id),
            Some(&task.id),
            &attempt,
        )?;
        transaction
            .execute(
                "INSERT INTO development_attempts(
                    id,development_task_id,attempt_number,state,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,1,?6)",
                params![
                    attempt.id,
                    task.id,
                    attempt.attempt_number,
                    attempt.state,
                    attempt.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(attempt)
    }

    pub(crate) fn reconcile_once(&mut self) -> Result<(), CleanStoreError> {
        let _ = self.expire_deadlines(&unix_timestamp()?)?;
        self.prune_page_snapshots()?;
        Ok(())
    }

    pub(crate) fn expire_deadlines(&mut self, now: &Timestamp) -> Result<usize, CleanStoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT payload FROM development_tasks
                 WHERE state IN (?1,?2,?3) AND deadline_at_seconds IS NOT NULL
                   AND (deadline_at_seconds<?4
                        OR (deadline_at_seconds=?4 AND deadline_at_nanos<=?5))
                 ORDER BY deadline_at_seconds,deadline_at_nanos,id LIMIT ?6",
            )
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement
            .query(params![
                v1::DevelopmentTaskState::Queued as i32,
                v1::DevelopmentTaskState::WaitingForRuntime as i32,
                v1::DevelopmentTaskState::Running as i32,
                now.seconds,
                now.nanos,
                i64::try_from(MAX_RECONCILE_BATCH).map_err(|_| CleanStoreError::CorruptState)?,
            ])
            .map_err(CleanStoreError::Database)?;
        let tasks = collect_bounded_payloads(&mut rows)?;
        drop(rows);
        drop(statement);
        let mut expired = 0_usize;
        for payload in tasks {
            let mut task: v1::DevelopmentTask = decode(&payload)?;
            let due = task
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.deadline_at.as_ref())
                .is_some_and(|deadline| timestamp_at_or_before(deadline, now));
            if !due {
                continue;
            }
            let previous_task_version = task.resource_version;
            task.state = v1::DevelopmentTaskState::DeadlineExceeded as i32;
            task.failure = Some(v1::ResourceFailure {
                reason: v1::ErrorReason::TaskDeadlineExpired as i32,
                message: "the development task deadline expired".to_owned(),
                remediation: None,
                observed_at: Some(now.to_owned()),
            });
            task.updated_at = Some(now.to_owned());
            task.resource_version = task.resource_version.saturating_add(1);
            let mut attempt = task
                .active_attempt_id
                .as_deref()
                .map(|id| self.development_attempt(id))
                .transpose()?;
            if let Some(value) = attempt.as_mut()
                && !is_terminal_attempt(value.state)
            {
                value.state = v1::DevelopmentAttemptState::DeadlineExceeded as i32;
                value.failure.clone_from(&task.failure);
                value.updated_at = Some(now.to_owned());
                value.completed_at = Some(now.to_owned());
                value.resource_version = value.resource_version.saturating_add(1);
            }
            let transaction = self
                .connection
                .transaction()
                .map_err(CleanStoreError::Database)?;
            persist_task_update(&transaction, &task, previous_task_version)?;
            if let Some(value) = attempt {
                persist_attempt_update(
                    &transaction,
                    &value,
                    value.resource_version.saturating_sub(1),
                    &task,
                )?;
            }
            transaction.commit().map_err(CleanStoreError::Database)?;
            expired = expired.saturating_add(1);
        }
        Ok(expired)
    }

    pub(crate) fn transition_attempt(
        &mut self,
        attempt_id: &str,
        expected_version: u64,
        new_state: v1::DevelopmentAttemptState,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::DevelopmentAttempt, CleanStoreError> {
        let mut attempt = self.development_attempt(attempt_id)?;
        let current = v1::DevelopmentAttemptState::try_from(attempt.state)
            .map_err(|_| CleanStoreError::CorruptState)?;
        let failure = bounded_failure(failure);
        if current == new_state
            && equivalent_failure(attempt.failure.as_ref(), failure.as_ref())
            && (attempt.resource_version == expected_version
                || attempt.resource_version == expected_version.saturating_add(1))
        {
            return Ok(attempt);
        }
        if attempt.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        if !valid_attempt_transition(current, new_state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        let now = unix_seconds()?;
        attempt.state = new_state as i32;
        attempt.failure = failure;
        attempt.updated_at = Some(timestamp(now));
        attempt.resource_version = attempt.resource_version.saturating_add(1);
        if is_terminal_attempt(attempt.state) {
            attempt.completed_at = Some(timestamp(now));
        }
        let mut task = self.development_task(&attempt.development_task_id)?;
        let previous_task_version = task.resource_version;
        if task.active_attempt_id.as_deref() == Some(attempt_id) {
            task.state = task_state_for_attempt(new_state) as i32;
            task.failure.clone_from(&attempt.failure);
            task.updated_at = Some(timestamp(now));
            task.resource_version = task.resource_version.saturating_add(1);
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        persist_attempt_update(&transaction, &attempt, expected_version, &task)?;
        if task.resource_version != previous_task_version {
            persist_task_update(&transaction, &task, previous_task_version)?;
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(attempt)
    }

    pub(crate) fn record_candidate(
        &mut self,
        mut candidate: v1::Candidate,
    ) -> Result<v1::Candidate, CleanStoreError> {
        candidate.failure = bounded_failure(candidate.failure);
        if !valid_candidate_report(&candidate) {
            return Err(CleanStoreError::BoundExceeded);
        }
        let existing: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT payload FROM candidates WHERE development_attempt_id=?1",
                [&candidate.development_attempt_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        if let Some(payload) = existing {
            let existing = decode::<v1::Candidate>(&payload)?;
            return if same_candidate_report(&existing, &candidate) {
                Ok(existing)
            } else {
                Err(CleanStoreError::IdempotencyConflict)
            };
        }
        let attempt = self.development_attempt(&candidate.development_attempt_id)?;
        if attempt.state != v1::DevelopmentAttemptState::Succeeded as i32 {
            return Err(CleanStoreError::InvalidTransition);
        }
        let task = self.development_task(&attempt.development_task_id)?;
        let repository_id = task
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.repository.as_ref())
            .map(|repository| repository.repository_connection_id.clone())
            .ok_or(CleanStoreError::CorruptState)?;
        if candidate.repository_connection_id != repository_id {
            return Err(CleanStoreError::ParentMismatch);
        }
        if candidate.id.is_empty() {
            candidate.id = resource_id("candidate");
        }
        candidate.resource_version = 1;
        if candidate.created_at.is_none() {
            candidate.created_at = Some(timestamp(unix_seconds()?));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "candidate",
            &candidate.id,
            1,
            Some(&repository_id),
            Some(&task.id),
            &candidate,
        )?;
        transaction
            .execute(
                "INSERT INTO candidates(
                    id,development_task_id,development_attempt_id,repository_connection_id,
                    state,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,?6,1,?7)",
                params![
                    candidate.id,
                    task.id,
                    attempt.id,
                    repository_id,
                    candidate.state,
                    candidate.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(candidate)
    }

    pub(crate) fn candidate(&self, id: &str) -> Result<v1::Candidate, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM candidates WHERE id=?1",
            id,
            "candidate",
        )
    }

    pub(crate) fn reject_candidate(
        &mut self,
        id: &str,
        expected_version: u64,
        failure: v1::ResourceFailure,
    ) -> Result<v1::Candidate, CleanStoreError> {
        let mut resource = self.candidate(id)?;
        if resource.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let current = v1::CandidateState::try_from(resource.state)
            .map_err(|_| CleanStoreError::CorruptState)?;
        if !valid_candidate_transition(current, v1::CandidateState::Rejected) {
            return Err(CleanStoreError::InvalidTransition);
        }
        let task_id: String = self
            .connection
            .query_row(
                "SELECT development_task_id FROM candidates WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        let task = self.development_task(&task_id)?;
        let repository_id = task
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.repository.as_ref())
            .map(|repository| repository.repository_connection_id.as_str())
            .ok_or(CleanStoreError::CorruptState)?;
        resource.state = v1::CandidateState::Rejected as i32;
        resource.failure = bounded_failure(Some(failure));
        resource.resource_version = resource.resource_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "candidate",
            &resource.id,
            resource.resource_version,
            Some(repository_id),
            Some(&task_id),
            &resource,
        )?;
        let changed = transaction
            .execute(
                "UPDATE candidates SET state=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
                params![
                    resource.state,
                    resource.encode_to_vec(),
                    i64::try_from(resource.resource_version)
                        .map_err(|_| CleanStoreError::CorruptState)?,
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                    resource.id,
                    i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(resource)
    }

    pub(crate) fn list_candidates(
        &mut self,
        task_id: Option<&str>,
        attempt_id: Option<&str>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::Candidate>, CleanStoreError> {
        let filter = encode_filter(&[task_id.unwrap_or(""), attempt_id.unwrap_or("")]);
        self.page_messages(
            "ListCandidates",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM candidates
                         WHERE (?1 IS NULL OR development_task_id=?1)
                           AND (?2 IS NULL OR development_attempt_id=?2)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![task_id, attempt_id])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            DEVELOPMENT_SERVICE,
        )
    }

    pub(crate) fn publish_candidate(
        &mut self,
        key: &str,
        request: &v1::PublishCandidateRequest,
    ) -> Result<v1::ChangeRequest, CleanStoreError> {
        let digest = request_digest("PublishCandidate", request);
        if let Some(existing) = action_row(&self.connection, "PublishCandidate", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return self.change_request(&existing.target_id.ok_or(CleanStoreError::CorruptState)?);
        }
        let candidate = self.candidate(&request.candidate_id)?;
        if candidate.state != v1::CandidateState::Available as i32 {
            return Err(CleanStoreError::InvalidTransition);
        }
        let head_revision = match candidate
            .identity
            .as_ref()
            .and_then(|value| value.identity.as_ref())
        {
            Some(v1::candidate_identity::Identity::Git(git)) => git.commit_sha.clone(),
            _ => return Err(CleanStoreError::InvalidTransition),
        };
        let now = unix_seconds()?;
        let id = resource_id("change-request");
        let change_request = v1::ChangeRequest {
            id: id.clone(),
            repository_connection_id: candidate.repository_connection_id.clone(),
            candidate_id: candidate.id,
            provider: None,
            base_revision: candidate.base_revision,
            head_revision,
            state: v1::ChangeRequestState::Publishing as i32,
            failure: None,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('PublishCandidate',?1,?2,?3,'pending','change_request',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), id, now],
            )
            .map_err(map_constraint)?;
        let cursor = append_event(
            &transaction,
            REVIEW_SERVICE,
            "change_request",
            &id,
            1,
            Some(&change_request.repository_connection_id),
            Some(&id),
            &change_request,
        )?;
        transaction
            .execute(
                "INSERT INTO change_requests(
                    id,repository_connection_id,candidate_id,state,head_revision,payload,
                    resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,?6,1,?7)",
                params![
                    id,
                    change_request.repository_connection_id,
                    change_request.candidate_id,
                    change_request.state,
                    change_request.head_revision,
                    change_request.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(change_request)
    }

    pub(crate) fn change_request(&self, id: &str) -> Result<v1::ChangeRequest, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM change_requests WHERE id=?1",
            id,
            "change_request",
        )
    }

    pub(crate) fn reconcile_change_request(
        &mut self,
        operation_key: &str,
        expected_version: u64,
        provider: v1::ChangeRequestProvider,
        state: v1::ChangeRequestState,
        head_revision: &str,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::ChangeRequest, CleanStoreError> {
        let action = self
            .durable_action("PublishCandidate", operation_key)?
            .ok_or(CleanStoreError::InvalidTransition)?;
        let id = action.target_id.ok_or(CleanStoreError::CorruptState)?;
        let mut change_request = self.change_request(&id)?;
        let current = v1::ChangeRequestState::try_from(change_request.state)
            .map_err(|_| CleanStoreError::CorruptState)?;
        let failure = bounded_failure(failure);
        if current == state
            && change_request.provider.as_ref() == Some(&provider)
            && change_request.head_revision == head_revision
            && equivalent_failure(change_request.failure.as_ref(), failure.as_ref())
            && (change_request.resource_version == expected_version
                || change_request.resource_version == expected_version.saturating_add(1))
        {
            return Ok(change_request);
        }
        if change_request.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        if !valid_change_request_transition(current, state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        let previous_version = change_request.resource_version;
        change_request.provider = Some(provider);
        change_request.state = state as i32;
        head_revision.clone_into(&mut change_request.head_revision);
        change_request.failure = failure;
        change_request.updated_at = Some(timestamp(unix_seconds()?));
        change_request.resource_version = previous_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        persist_change_request_update(&transaction, &change_request, previous_version)?;
        transaction
            .execute(
                "UPDATE durable_actions SET state=?1,failure=?2,updated_at=?3
                 WHERE operation='PublishCandidate' AND idempotency_key=?4",
                params![
                    if state == v1::ChangeRequestState::Failed {
                        "failed"
                    } else {
                        "completed"
                    },
                    change_request
                        .failure
                        .as_ref()
                        .map(|value| truncate_utf8(&value.message, 512)),
                    unix_seconds()?,
                    operation_key
                ],
            )
            .map_err(CleanStoreError::Database)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(change_request)
    }

    pub(crate) fn retry_change_request_publication(
        &mut self,
        operation_key: &str,
        expected_version: u64,
    ) -> Result<v1::ChangeRequest, CleanStoreError> {
        let action = self
            .durable_action("PublishCandidate", operation_key)?
            .ok_or(CleanStoreError::InvalidTransition)?;
        if action.state != DurableActionState::Failed {
            return Err(CleanStoreError::InvalidTransition);
        }
        let id = action.target_id.ok_or(CleanStoreError::CorruptState)?;
        let mut change_request = self.change_request(&id)?;
        if change_request.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        if change_request.state != v1::ChangeRequestState::Failed as i32 {
            return Err(CleanStoreError::InvalidTransition);
        }
        let previous_version = change_request.resource_version;
        change_request.state = v1::ChangeRequestState::Publishing as i32;
        change_request.failure = None;
        change_request.updated_at = Some(timestamp(unix_seconds()?));
        change_request.resource_version = previous_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        persist_change_request_update(&transaction, &change_request, previous_version)?;
        let changed = transaction
            .execute(
                "UPDATE durable_actions SET state='pending',failure=NULL,updated_at=?1
                 WHERE operation='PublishCandidate' AND idempotency_key=?2 AND state='failed'",
                params![unix_seconds()?, operation_key],
            )
            .map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::InvalidTransition);
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(change_request)
    }

    pub(crate) fn list_change_requests(
        &mut self,
        repository_connection_id: Option<&str>,
        candidate_id: Option<&str>,
        state: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::ChangeRequest>, CleanStoreError> {
        let state_text = state.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[
            repository_connection_id.unwrap_or(""),
            candidate_id.unwrap_or(""),
            &state_text,
        ]);
        self.page_messages(
            "ListChangeRequests",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM change_requests
                         WHERE (?1 IS NULL OR repository_connection_id=?1)
                           AND (?2 IS NULL OR candidate_id=?2)
                           AND (?3 IS NULL OR state=?3)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![repository_connection_id, candidate_id, state])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            REVIEW_SERVICE,
        )
    }

    pub(crate) fn record_assessment(
        &mut self,
        mut assessment: v1::Assessment,
    ) -> Result<v1::Assessment, CleanStoreError> {
        if !valid_assessment_report(&assessment) {
            return Err(CleanStoreError::BoundExceeded);
        }
        let change_request = self.change_request(&assessment.change_request_id)?;
        if assessment.repository_connection_id != change_request.repository_connection_id {
            return Err(CleanStoreError::ParentMismatch);
        }
        if assessment.id.is_empty() {
            assessment.id = resource_id("assessment");
        }
        assessment.resource_version = 1;
        if assessment.created_at.is_none() {
            assessment.created_at = Some(timestamp(unix_seconds()?));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            REVIEW_SERVICE,
            "assessment",
            &assessment.id,
            1,
            Some(&assessment.repository_connection_id),
            Some(&assessment.change_request_id),
            &assessment,
        )?;
        transaction
            .execute(
                "INSERT INTO assessments(
                    id,repository_connection_id,change_request_id,head_revision,conclusion,
                    payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,?6,1,?7)",
                params![
                    assessment.id,
                    assessment.repository_connection_id,
                    assessment.change_request_id,
                    assessment.head_revision,
                    assessment.conclusion,
                    assessment.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(assessment)
    }

    pub(crate) fn assessment(&self, id: &str) -> Result<v1::Assessment, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM assessments WHERE id=?1",
            id,
            "assessment",
        )
    }

    pub(crate) fn list_assessments(
        &mut self,
        change_request_id: &str,
        head_revision: Option<&str>,
        conclusion: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::Assessment>, CleanStoreError> {
        self.change_request(change_request_id)?;
        let conclusion_text = conclusion.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[
            change_request_id,
            head_revision.unwrap_or(""),
            &conclusion_text,
        ]);
        self.page_messages(
            "ListAssessments",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM assessments
                         WHERE change_request_id=?1
                           AND (?2 IS NULL OR head_revision=?2)
                           AND (?3 IS NULL OR conclusion=?3)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![change_request_id, head_revision, conclusion])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            REVIEW_SERVICE,
        )
    }

    pub(crate) fn record_evidence_reference(
        &mut self,
        mut evidence: v1::EvidenceReference,
    ) -> Result<v1::EvidenceReference, CleanStoreError> {
        if !valid_evidence_report(&evidence) {
            return Err(CleanStoreError::BoundExceeded);
        }
        let assessment = self.assessment(&evidence.assessment_id)?;
        if evidence.id.is_empty() {
            evidence.id = resource_id("evidence");
        }
        evidence.resource_version = 1;
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            REVIEW_SERVICE,
            "evidence_reference",
            &evidence.id,
            1,
            Some(&assessment.repository_connection_id),
            Some(&assessment.change_request_id),
            &evidence,
        )?;
        transaction
            .execute(
                "INSERT INTO evidence_references(
                    id,assessment_id,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,1,?4)",
                params![
                    evidence.id,
                    evidence.assessment_id,
                    evidence.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(evidence)
    }

    pub(crate) fn evidence_reference(
        &self,
        id: &str,
    ) -> Result<v1::EvidenceReference, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM evidence_references WHERE id=?1",
            id,
            "evidence_reference",
        )
    }

    pub(crate) fn list_evidence_references(
        &mut self,
        assessment_id: &str,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::EvidenceReference>, CleanStoreError> {
        self.assessment(assessment_id)?;
        let filter = encode_filter(&[assessment_id]);
        self.page_messages(
            "ListEvidenceReferences",
            &filter,
            page,
            |connection| {
                query_payloads(
                    connection,
                    "evidence_references",
                    "assessment_id",
                    None,
                    Some(assessment_id),
                )
            },
            REVIEW_SERVICE,
        )
    }
}

fn validate_indexed_rows<M, F>(
    connection: &Connection,
    sql: &str,
    validate: F,
) -> Result<(), CleanStoreError>
where
    M: Message + Default,
    F: Fn(&rusqlite::Row<'_>, &M) -> rusqlite::Result<bool>,
{
    let mut statement = connection.prepare(sql).map_err(CleanStoreError::Database)?;
    let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
    while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
        let payload: Vec<u8> = row.get(0).map_err(CleanStoreError::Database)?;
        let value = decode::<M>(&payload)?;
        if !validate(row, &value).map_err(CleanStoreError::Database)? {
            return Err(CleanStoreError::CorruptState);
        }
    }
    Ok(())
}

fn valid_version_and_cursor(version: u64, indexed_version: i64, cursor: i64) -> bool {
    version > 0 && i64::try_from(version) == Ok(indexed_version) && cursor > 0
}

fn validate_resource_tables(connection: &Connection) -> Result<(), CleanStoreError> {
    validate_indexed_rows::<v1::RepositoryConnection, _>(
        connection,
        "SELECT payload,id,status,resource_version,event_cursor FROM repository_connections",
        |row, value| {
            Ok(!value.id.is_empty()
                && value.provider.is_some()
                && value.status != v1::RepositoryConnectionStatus::Unspecified as i32
                && v1::RepositoryConnectionStatus::try_from(value.status).is_ok()
                && value.id == row.get::<_, String>(1)?
                && value.status == row.get::<_, i32>(2)?
                && valid_version_and_cursor(value.resource_version, row.get(3)?, row.get(4)?))
        },
    )?;
    validate_indexed_rows::<v1::RepositorySnapshot, _>(
        connection,
        "SELECT payload,id,repository_connection_id,revision,resource_version,event_cursor
         FROM repository_snapshots",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.repository_connection_id.is_empty()
                && !value.revision.is_empty()
                && value.id == row.get::<_, String>(1)?
                && value.repository_connection_id == row.get::<_, String>(2)?
                && value.revision == row.get::<_, String>(3)?
                && valid_version_and_cursor(value.resource_version, row.get(4)?, row.get(5)?))
        },
    )?;
    validate_indexed_rows::<v1::RuntimeProfile, _>(
        connection,
        "SELECT payload,id,repository_connection_id,kind,state,active_revision_id,
                active_runtime_artifact_id,resource_version,event_cursor FROM runtime_profiles",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.repository_connection_id.is_empty()
                && value.kind != v1::RuntimeProfileKind::Unspecified as i32
                && value.support != v1::RuntimeProfileSupport::Unspecified as i32
                && value.state != v1::RuntimeProfileState::Unspecified as i32
                && v1::RuntimeProfileKind::try_from(value.kind).is_ok()
                && v1::RuntimeProfileSupport::try_from(value.support).is_ok()
                && v1::RuntimeProfileState::try_from(value.state).is_ok()
                && value.id == row.get::<_, String>(1)?
                && value.repository_connection_id == row.get::<_, String>(2)?
                && value.kind == row.get::<_, i32>(3)?
                && value.state == row.get::<_, i32>(4)?
                && value.active_revision_id == row.get::<_, Option<String>>(5)?
                && value.active_runtime_artifact_id == row.get::<_, Option<String>>(6)?
                && valid_version_and_cursor(value.resource_version, row.get(7)?, row.get(8)?))
        },
    )?;
    validate_indexed_rows::<v1::RuntimeProfileRevision, _>(
        connection,
        "SELECT payload,id,runtime_profile_id,repository_snapshot_id,revision_number,
                resource_version,event_cursor FROM runtime_profile_revisions",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.runtime_profile_id.is_empty()
                && !value.repository_snapshot_id.is_empty()
                && value.revision_number > 0
                && value.configuration.is_some()
                && value.id == row.get::<_, String>(1)?
                && value.runtime_profile_id == row.get::<_, String>(2)?
                && value.repository_snapshot_id == row.get::<_, String>(3)?
                && i64::try_from(value.revision_number) == Ok(row.get::<_, i64>(4)?)
                && valid_version_and_cursor(value.resource_version, row.get(5)?, row.get(6)?))
        },
    )?;
    validate_indexed_rows::<v1::RuntimeArtifact, _>(
        connection,
        "SELECT payload,id,runtime_profile_id,runtime_profile_revision_id,status,resolved_digest,
                resource_version,event_cursor FROM runtime_artifacts",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.runtime_profile_revision_id.is_empty()
                && !value.resolved_digest.is_empty()
                && value.platform.is_some()
                && value.status != v1::RuntimeArtifactStatus::Unspecified as i32
                && v1::RuntimeArtifactStatus::try_from(value.status).is_ok()
                && valid_artifact_report(value)
                && value.id == row.get::<_, String>(1)?
                && value.runtime_profile_revision_id == row.get::<_, String>(3)?
                && value.status == row.get::<_, i32>(4)?
                && value.resolved_digest == row.get::<_, String>(5)?
                && valid_version_and_cursor(value.resource_version, row.get(6)?, row.get(7)?)
                && !row.get::<_, String>(2)?.is_empty())
        },
    )?;
    validate_indexed_rows::<v1::AutomationInstallation, _>(
        connection,
        "SELECT payload,id,runtime_profile_id,state,version,resource_version,event_cursor
         FROM automation_installations",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.runtime_profile_id.is_empty()
                && !value.version.is_empty()
                && value.state != v1::AutomationInstallationState::Unspecified as i32
                && v1::AutomationInstallationState::try_from(value.state).is_ok()
                && value.id == row.get::<_, String>(1)?
                && value.runtime_profile_id == row.get::<_, String>(2)?
                && value.state == row.get::<_, i32>(3)?
                && value.version == row.get::<_, String>(4)?
                && valid_version_and_cursor(value.resource_version, row.get(5)?, row.get(6)?))
        },
    )?;
    validate_indexed_rows::<v1::DevelopmentTask, _>(
        connection,
        "SELECT payload,id,state,active_attempt_id,deadline_at_seconds,deadline_at_nanos,
                resource_version,event_cursor FROM development_tasks",
        |row, value| {
            let deadline = value
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.deadline_at.as_ref());
            Ok(!value.id.is_empty()
                && value.snapshot.is_some()
                && value.attempt_count > 0
                && value.state != v1::DevelopmentTaskState::Unspecified as i32
                && v1::DevelopmentTaskState::try_from(value.state).is_ok()
                && value.id == row.get::<_, String>(1)?
                && value.state == row.get::<_, i32>(2)?
                && value.active_attempt_id == row.get::<_, Option<String>>(3)?
                && deadline.map(|value| value.seconds) == row.get::<_, Option<i64>>(4)?
                && deadline.map(|value| value.nanos) == row.get::<_, Option<i32>>(5)?
                && valid_version_and_cursor(value.resource_version, row.get(6)?, row.get(7)?))
        },
    )?;
    validate_indexed_rows::<v1::DevelopmentAttempt, _>(
        connection,
        "SELECT payload,id,development_task_id,attempt_number,state,resource_version,event_cursor
         FROM development_attempts",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.development_task_id.is_empty()
                && value.attempt_number > 0
                && value.snapshot.is_some()
                && value.state != v1::DevelopmentAttemptState::Unspecified as i32
                && v1::DevelopmentAttemptState::try_from(value.state).is_ok()
                && valid_model_usage(value.model_usage.as_ref())
                && value.id == row.get::<_, String>(1)?
                && value.development_task_id == row.get::<_, String>(2)?
                && i64::from(value.attempt_number) == row.get::<_, i64>(3)?
                && value.state == row.get::<_, i32>(4)?
                && valid_version_and_cursor(value.resource_version, row.get(5)?, row.get(6)?))
        },
    )?;
    validate_indexed_rows::<v1::Candidate, _>(
        connection,
        "SELECT payload,id,development_attempt_id,repository_connection_id,state,
                resource_version,event_cursor FROM candidates",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.development_attempt_id.is_empty()
                && !value.repository_connection_id.is_empty()
                && value.identity.is_some()
                && value.state != v1::CandidateState::Unspecified as i32
                && v1::CandidateState::try_from(value.state).is_ok()
                && valid_candidate_report(value)
                && value.id == row.get::<_, String>(1)?
                && value.development_attempt_id == row.get::<_, String>(2)?
                && value.repository_connection_id == row.get::<_, String>(3)?
                && value.state == row.get::<_, i32>(4)?
                && valid_version_and_cursor(value.resource_version, row.get(5)?, row.get(6)?))
        },
    )?;
    validate_indexed_rows::<v1::ChangeRequest, _>(
        connection,
        "SELECT payload,id,repository_connection_id,candidate_id,state,head_revision,
                resource_version,event_cursor FROM change_requests",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.repository_connection_id.is_empty()
                && !value.candidate_id.is_empty()
                && !value.head_revision.is_empty()
                && value.state != v1::ChangeRequestState::Unspecified as i32
                && v1::ChangeRequestState::try_from(value.state).is_ok()
                && value.id == row.get::<_, String>(1)?
                && value.repository_connection_id == row.get::<_, String>(2)?
                && value.candidate_id == row.get::<_, String>(3)?
                && value.state == row.get::<_, i32>(4)?
                && value.head_revision == row.get::<_, String>(5)?
                && valid_version_and_cursor(value.resource_version, row.get(6)?, row.get(7)?))
        },
    )?;
    validate_indexed_rows::<v1::Assessment, _>(
        connection,
        "SELECT payload,id,repository_connection_id,change_request_id,head_revision,conclusion,
                resource_version,event_cursor FROM assessments",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.repository_connection_id.is_empty()
                && !value.change_request_id.is_empty()
                && !value.head_revision.is_empty()
                && value.producer.is_some()
                && value.workflow.is_some()
                && value.conclusion != v1::AssessmentConclusion::Unspecified as i32
                && v1::AssessmentConclusion::try_from(value.conclusion).is_ok()
                && valid_assessment_report(value)
                && value.id == row.get::<_, String>(1)?
                && value.repository_connection_id == row.get::<_, String>(2)?
                && value.change_request_id == row.get::<_, String>(3)?
                && value.head_revision == row.get::<_, String>(4)?
                && value.conclusion == row.get::<_, i32>(5)?
                && valid_version_and_cursor(value.resource_version, row.get(6)?, row.get(7)?))
        },
    )?;
    validate_indexed_rows::<v1::EvidenceReference, _>(
        connection,
        "SELECT payload,id,assessment_id,resource_version,event_cursor FROM evidence_references",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.assessment_id.is_empty()
                && !value.name.is_empty()
                && !value.digest.is_empty()
                && valid_evidence_report(value)
                && value.id == row.get::<_, String>(1)?
                && value.assessment_id == row.get::<_, String>(2)?
                && valid_version_and_cursor(value.resource_version, row.get(3)?, row.get(4)?))
        },
    )?;
    Ok(())
}

fn validate_durable_actions(connection: &Connection) -> Result<(), CleanStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,failure
             FROM durable_actions",
        )
        .map_err(CleanStoreError::Database)?;
    let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
    while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
        let operation: String = row.get(0).map_err(CleanStoreError::Database)?;
        let key: String = row.get(1).map_err(CleanStoreError::Database)?;
        let hash: Vec<u8> = row.get(2).map_err(CleanStoreError::Database)?;
        let payload: Vec<u8> = row.get(3).map_err(CleanStoreError::Database)?;
        let state: String = row.get(4).map_err(CleanStoreError::Database)?;
        let target_type: Option<String> = row.get(5).map_err(CleanStoreError::Database)?;
        let target_id: Option<String> = row.get(6).map_err(CleanStoreError::Database)?;
        let failure: Option<String> = row.get(7).map_err(CleanStoreError::Database)?;
        let mut hasher = Sha256::new();
        hasher.update(b"ayni.orchestrator.action.v1\0");
        hasher.update(operation.as_bytes());
        hasher.update([0]);
        hasher.update(&payload);
        if operation.is_empty()
            || key.is_empty()
            || hasher.finalize().as_slice() != hash
            || !action_payload_is_valid(&operation, &payload)
            || !matches!(state.as_str(), "pending" | "completed" | "failed")
            || target_type.as_deref().is_none_or(str::is_empty)
            || target_id.as_deref().is_none_or(str::is_empty)
            || (state == "failed"
                && failure
                    .as_deref()
                    .is_none_or(|value| value.is_empty() || value.len() > 512))
            || (state != "failed" && failure.is_some())
        {
            return Err(CleanStoreError::CorruptState);
        }
    }
    Ok(())
}

fn action_payload_is_valid(operation: &str, payload: &[u8]) -> bool {
    match operation {
        "CreateRepositoryConnection" => v1::CreateRepositoryConnectionRequest::decode(payload)
            .is_ok_and(|value| api_validation::create_repository_connection(&value).is_ok()),
        "CreateRuntimeProfile" => v1::CreateRuntimeProfileRequest::decode(payload)
            .is_ok_and(|value| api_validation::create_runtime_profile(&value).is_ok()),
        "InstallRepositoryAutomation" => v1::InstallRepositoryAutomationRequest::decode(payload)
            .is_ok_and(|value| {
                api_validation::idempotency_key(&value.idempotency_key).is_ok()
                    && api_validation::resource_id(&value.runtime_profile_id, "runtime_profile_id")
                        .is_ok()
            }),
        "ReconcileRuntimeProfile" => {
            v1::ReconcileRuntimeProfileRequest::decode(payload).is_ok_and(|value| {
                api_validation::idempotency_key(&value.idempotency_key).is_ok()
                    && api_validation::resource_id(&value.runtime_profile_id, "runtime_profile_id")
                        .is_ok()
            })
        }
        "CreateDevelopmentTask" => v1::CreateDevelopmentTaskRequest::decode(payload)
            .is_ok_and(|value| api_validation::create_development_task(&value).is_ok()),
        "CancelDevelopmentTask" => {
            v1::CancelDevelopmentTaskRequest::decode(payload).is_ok_and(|value| {
                api_validation::idempotency_key(&value.idempotency_key).is_ok()
                    && api_validation::resource_id(
                        &value.development_task_id,
                        "development_task_id",
                    )
                    .is_ok()
            })
        }
        "RetryDevelopmentTask" => {
            v1::RetryDevelopmentTaskRequest::decode(payload).is_ok_and(|value| {
                api_validation::idempotency_key(&value.idempotency_key).is_ok()
                    && api_validation::resource_id(
                        &value.development_task_id,
                        "development_task_id",
                    )
                    .is_ok()
            })
        }
        "PublishCandidate" => v1::PublishCandidateRequest::decode(payload)
            .is_ok_and(|value| api_validation::publish_candidate(&value).is_ok()),
        _ => false,
    }
}

fn validate_resource_events(connection: &Connection) -> Result<(), CleanStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT service,resource_type,resource_id,resource_version,payload
             FROM resource_events ORDER BY cursor",
        )
        .map_err(CleanStoreError::Database)?;
    let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
    while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
        let service: String = row.get(0).map_err(CleanStoreError::Database)?;
        let resource_type: String = row.get(1).map_err(CleanStoreError::Database)?;
        let resource_id: String = row.get(2).map_err(CleanStoreError::Database)?;
        let version: i64 = row.get(3).map_err(CleanStoreError::Database)?;
        let payload: Vec<u8> = row.get(4).map_err(CleanStoreError::Database)?;
        let valid = match (service.as_str(), resource_type.as_str()) {
            (REPOSITORY_SERVICE, "repository_connection") => {
                let value = decode::<v1::RepositoryConnection>(&payload)?;
                value.id == resource_id
                    && value.provider.is_some()
                    && value.status != v1::RepositoryConnectionStatus::Unspecified as i32
                    && v1::RepositoryConnectionStatus::try_from(value.status).is_ok()
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            (REPOSITORY_SERVICE, "repository_snapshot") => {
                let value = decode::<v1::RepositorySnapshot>(&payload)?;
                value.id == resource_id
                    && !value.repository_connection_id.is_empty()
                    && !value.revision.is_empty()
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            (PROFILE_SERVICE, "runtime_profile") => {
                let value = decode::<v1::RuntimeProfile>(&payload)?;
                value.id == resource_id
                    && !value.repository_connection_id.is_empty()
                    && value.kind != v1::RuntimeProfileKind::Unspecified as i32
                    && value.support != v1::RuntimeProfileSupport::Unspecified as i32
                    && value.state != v1::RuntimeProfileState::Unspecified as i32
                    && v1::RuntimeProfileKind::try_from(value.kind).is_ok()
                    && v1::RuntimeProfileSupport::try_from(value.support).is_ok()
                    && v1::RuntimeProfileState::try_from(value.state).is_ok()
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            (PROFILE_SERVICE, "runtime_profile_revision") => {
                let value = decode::<v1::RuntimeProfileRevision>(&payload)?;
                value.id == resource_id
                    && !value.runtime_profile_id.is_empty()
                    && !value.repository_snapshot_id.is_empty()
                    && value.revision_number > 0
                    && value.configuration.is_some()
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            (PROFILE_SERVICE, "runtime_artifact") => {
                let value = decode::<v1::RuntimeArtifact>(&payload)?;
                value.id == resource_id
                    && valid_artifact_report(&value)
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            (PROFILE_SERVICE, "automation_installation") => {
                let value = decode::<v1::AutomationInstallation>(&payload)?;
                value.id == resource_id
                    && !value.runtime_profile_id.is_empty()
                    && !value.version.is_empty()
                    && value.state != v1::AutomationInstallationState::Unspecified as i32
                    && v1::AutomationInstallationState::try_from(value.state).is_ok()
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            (DEVELOPMENT_SERVICE, "development_task") => {
                let value = decode::<v1::DevelopmentTask>(&payload)?;
                value.id == resource_id
                    && value.snapshot.is_some()
                    && value.attempt_count > 0
                    && value.state != v1::DevelopmentTaskState::Unspecified as i32
                    && v1::DevelopmentTaskState::try_from(value.state).is_ok()
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            (DEVELOPMENT_SERVICE, "development_attempt") => {
                let value = decode::<v1::DevelopmentAttempt>(&payload)?;
                value.id == resource_id
                    && !value.development_task_id.is_empty()
                    && value.attempt_number > 0
                    && value.snapshot.is_some()
                    && value.state != v1::DevelopmentAttemptState::Unspecified as i32
                    && v1::DevelopmentAttemptState::try_from(value.state).is_ok()
                    && valid_model_usage(value.model_usage.as_ref())
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            (DEVELOPMENT_SERVICE, "candidate") => {
                let value = decode::<v1::Candidate>(&payload)?;
                value.id == resource_id
                    && valid_candidate_report(&value)
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            (REVIEW_SERVICE, "change_request") => {
                let value = decode::<v1::ChangeRequest>(&payload)?;
                value.id == resource_id
                    && !value.repository_connection_id.is_empty()
                    && !value.candidate_id.is_empty()
                    && valid_git_revision(&value.head_revision)
                    && value.state != v1::ChangeRequestState::Unspecified as i32
                    && v1::ChangeRequestState::try_from(value.state).is_ok()
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            (REVIEW_SERVICE, "assessment") => {
                let value = decode::<v1::Assessment>(&payload)?;
                value.id == resource_id
                    && valid_assessment_report(&value)
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            (REVIEW_SERVICE, "evidence_reference") => {
                let value = decode::<v1::EvidenceReference>(&payload)?;
                value.id == resource_id
                    && valid_evidence_report(&value)
                    && i64::try_from(value.resource_version) == Ok(version)
            }
            _ => false,
        };
        if resource_id.is_empty() || version <= 0 || !valid {
            return Err(CleanStoreError::CorruptState);
        }
    }
    Ok(())
}

fn validate_page_snapshot_rows(connection: &Connection) -> Result<(), CleanStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT method,filter_hash,cursor,items,token_secret,created_at FROM page_snapshots",
        )
        .map_err(CleanStoreError::Database)?;
    let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
    while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
        let method: String = row.get(0).map_err(CleanStoreError::Database)?;
        let filter_hash: String = row.get(1).map_err(CleanStoreError::Database)?;
        let cursor: i64 = row.get(2).map_err(CleanStoreError::Database)?;
        let encoded: String = row.get(3).map_err(CleanStoreError::Database)?;
        let token_secret: String = row.get(4).map_err(CleanStoreError::Database)?;
        let created_at: i64 = row.get(5).map_err(CleanStoreError::Database)?;
        let items: Vec<String> =
            serde_json::from_str(&encoded).map_err(|_| CleanStoreError::CorruptState)?;
        let decoded = items
            .iter()
            .map(|item| {
                URL_SAFE_NO_PAD
                    .decode(item)
                    .map_err(|_| CleanStoreError::CorruptState)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if method.is_empty()
            || filter_hash.len() != 64
            || cursor < 0
            || token_secret.is_empty()
            || created_at < 0
            || items.len() > MAX_PAGE_SNAPSHOT_ITEMS
            || encoded.len() > MAX_PAGE_SNAPSHOT_BYTES.saturating_mul(2)
            || decoded.iter().map(Vec::len).sum::<usize>() > MAX_PAGE_SNAPSHOT_BYTES
            || !page_items_decode(&method, &decoded)
        {
            return Err(CleanStoreError::CorruptState);
        }
    }
    Ok(())
}

fn get_payload<M: Message + Default>(
    connection: &Connection,
    sql: &str,
    id: &str,
    resource_type: &'static str,
) -> Result<M, CleanStoreError> {
    let payload = connection
        .query_row(sql, [id], |row| row.get::<_, Vec<u8>>(0))
        .optional()
        .map_err(CleanStoreError::Database)?
        .ok_or_else(|| CleanStoreError::NotFound {
            resource_type,
            resource_id: id.to_owned(),
        })?;
    decode(&payload)
}

fn collect_bounded_payloads(
    rows: &mut rusqlite::Rows<'_>,
) -> Result<Vec<Vec<u8>>, CleanStoreError> {
    let mut payloads = Vec::new();
    let mut bytes = 0_usize;
    while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
        let value = row.get_ref(0).map_err(CleanStoreError::Database)?;
        let ValueRef::Blob(payload) = value else {
            return Err(CleanStoreError::CorruptState);
        };
        let next_bytes = bytes
            .checked_add(payload.len())
            .ok_or(CleanStoreError::BoundExceeded)?;
        if payloads.len() >= MAX_PAGE_SNAPSHOT_ITEMS || next_bytes > MAX_PAGE_SNAPSHOT_BYTES {
            return Err(CleanStoreError::BoundExceeded);
        }
        payloads.push(payload.to_vec());
        bytes = next_bytes;
    }
    Ok(payloads)
}

fn query_payloads(
    connection: &Connection,
    table: &str,
    parent_column: &str,
    status: Option<i32>,
    parent: Option<&str>,
) -> Result<Vec<Vec<u8>>, CleanStoreError> {
    let sql = if parent_column.is_empty() {
        format!("SELECT payload FROM {table} WHERE (?1 IS NULL OR status=?1) ORDER BY id")
    } else {
        format!("SELECT payload FROM {table} WHERE {parent_column}=?1 ORDER BY id")
    };
    let mut statement = connection
        .prepare(&sql)
        .map_err(CleanStoreError::Database)?;
    if parent_column.is_empty() {
        let mut rows = statement
            .query([status])
            .map_err(CleanStoreError::Database)?;
        collect_bounded_payloads(&mut rows)
    } else {
        let mut rows = statement
            .query([parent])
            .map_err(CleanStoreError::Database)?;
        collect_bounded_payloads(&mut rows)
    }
}

fn persist_task_update(
    transaction: &Transaction<'_>,
    task: &v1::DevelopmentTask,
    expected_version: u64,
) -> Result<(), CleanStoreError> {
    let repository_id = task
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.repository.as_ref())
        .map(|repository| repository.repository_connection_id.as_str())
        .ok_or(CleanStoreError::CorruptState)?;
    let cursor = append_event(
        transaction,
        DEVELOPMENT_SERVICE,
        "development_task",
        &task.id,
        task.resource_version,
        Some(repository_id),
        Some(&task.id),
        task,
    )?;
    let changed = transaction
        .execute(
            "UPDATE development_tasks SET state=?1,active_attempt_id=?2,payload=?3,
                resource_version=?4,event_cursor=?5 WHERE id=?6 AND resource_version=?7",
            params![
                task.state,
                task.active_attempt_id,
                task.encode_to_vec(),
                i64::try_from(task.resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                task.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?
            ],
        )
        .map_err(CleanStoreError::Database)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(CleanStoreError::StaleVersion)
    }
}

fn persist_attempt_update(
    transaction: &Transaction<'_>,
    attempt: &v1::DevelopmentAttempt,
    expected_version: u64,
    task: &v1::DevelopmentTask,
) -> Result<(), CleanStoreError> {
    let repository_id = task
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.repository.as_ref())
        .map(|repository| repository.repository_connection_id.as_str())
        .ok_or(CleanStoreError::CorruptState)?;
    let cursor = append_event(
        transaction,
        DEVELOPMENT_SERVICE,
        "development_attempt",
        &attempt.id,
        attempt.resource_version,
        Some(repository_id),
        Some(&task.id),
        attempt,
    )?;
    let changed = transaction
        .execute(
            "UPDATE development_attempts SET state=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
            params![
                attempt.state,
                attempt.encode_to_vec(),
                i64::try_from(attempt.resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                attempt.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?
            ],
        )
        .map_err(CleanStoreError::Database)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(CleanStoreError::StaleVersion)
    }
}

fn persist_change_request_update(
    transaction: &Transaction<'_>,
    change_request: &v1::ChangeRequest,
    expected_version: u64,
) -> Result<(), CleanStoreError> {
    let cursor = append_event(
        transaction,
        REVIEW_SERVICE,
        "change_request",
        &change_request.id,
        change_request.resource_version,
        Some(&change_request.repository_connection_id),
        Some(&change_request.id),
        change_request,
    )?;
    let changed = transaction
        .execute(
            "UPDATE change_requests SET state=?1,head_revision=?2,payload=?3,
                resource_version=?4,event_cursor=?5 WHERE id=?6 AND resource_version=?7",
            params![
                change_request.state,
                change_request.head_revision,
                change_request.encode_to_vec(),
                i64::try_from(change_request.resource_version)
                    .map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                change_request.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?
            ],
        )
        .map_err(CleanStoreError::Database)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(CleanStoreError::StaleVersion)
    }
}

fn is_terminal_task(state: i32) -> bool {
    matches!(
        v1::DevelopmentTaskState::try_from(state).ok(),
        Some(
            v1::DevelopmentTaskState::Succeeded
                | v1::DevelopmentTaskState::Failed
                | v1::DevelopmentTaskState::Cancelled
                | v1::DevelopmentTaskState::DeadlineExceeded
                | v1::DevelopmentTaskState::Unsupported
        )
    )
}

fn is_terminal_attempt(state: i32) -> bool {
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

const fn valid_repository_transition(
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

const fn valid_profile_transition(
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
            State::Ready | State::Updating | State::Degraded | State::Failed
        ) | (State::Ready, State::Updating | State::Degraded)
            | (
                State::Updating,
                State::Ready | State::Degraded | State::Failed
            )
            | (
                State::Degraded,
                State::SetupRequired
                    | State::AwaitingEnvironment
                    | State::Updating
                    | State::Ready
                    | State::Failed
            )
    )
}

const fn valid_artifact_transition(
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

const fn valid_automation_transition(
    current: v1::AutomationInstallationState,
    next: v1::AutomationInstallationState,
) -> bool {
    use v1::AutomationInstallationState as State;
    matches!(
        (current, next),
        (
            State::SetupRequired,
            State::SetupPrOpen | State::Installed | State::Degraded | State::Failed
        ) | (
            State::SetupPrOpen,
            State::SetupRequired
                | State::Installed
                | State::Updating
                | State::Degraded
                | State::Failed
        ) | (State::Installed, State::Updating | State::Degraded)
            | (
                State::Updating,
                State::Installed | State::Degraded | State::Failed
            )
            | (
                State::Degraded,
                State::SetupRequired | State::Updating | State::Installed | State::Failed
            )
    )
}

const fn valid_candidate_transition(current: v1::CandidateState, next: v1::CandidateState) -> bool {
    matches!(
        (current, next),
        (v1::CandidateState::Available, v1::CandidateState::Rejected)
    )
}

const fn valid_attempt_transition(
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

const fn task_state_for_attempt(state: v1::DevelopmentAttemptState) -> v1::DevelopmentTaskState {
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

const fn valid_change_request_transition(
    current: v1::ChangeRequestState,
    next: v1::ChangeRequestState,
) -> bool {
    use v1::ChangeRequestState as State;
    matches!(
        (current, next),
        (
            State::Publishing,
            State::Draft | State::Open | State::Failed
        ) | (
            State::Draft,
            State::Open | State::Closed | State::Merged | State::Failed
        ) | (State::Open, State::Closed | State::Merged | State::Failed)
    )
}

const fn map_constraint(error: rusqlite::Error) -> CleanStoreError {
    CleanStoreError::Database(error)
}

#[derive(Debug)]
struct ActionRow {
    request_hash: Vec<u8>,
    request_payload: Vec<u8>,
    state: String,
    target_type: Option<String>,
    target_id: Option<String>,
    failure: Option<String>,
}

impl ActionRow {
    fn into_action(self, operation: &str, key: &str) -> DurableAction {
        let state = match self.state.as_str() {
            "completed" => DurableActionState::Completed,
            "failed" => DurableActionState::Failed,
            _ => DurableActionState::Pending,
        };
        DurableAction {
            operation: operation.to_owned(),
            idempotency_key: key.to_owned(),
            state,
            target_type: self.target_type,
            target_id: self.target_id,
            request_payload: self.request_payload,
            failure: self.failure,
        }
    }
}

fn action_row(
    connection: &Connection,
    operation: &str,
    key: &str,
) -> Result<Option<ActionRow>, CleanStoreError> {
    connection
        .query_row(
            "SELECT request_hash,request_payload,state,target_type,target_id,failure FROM durable_actions
             WHERE operation=?1 AND idempotency_key=?2",
            params![operation, key],
            |row| {
                Ok(ActionRow {
                    request_hash: row.get(0)?,
                    request_payload: row.get(1)?,
                    state: row.get(2)?,
                    target_type: row.get(3)?,
                    target_id: row.get(4)?,
                    failure: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(CleanStoreError::Database)
}

fn request_digest<M: Message>(operation: &str, request: &M) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(b"ayni.orchestrator.action.v1\0");
    hasher.update(operation.as_bytes());
    hasher.update([0]);
    hasher.update(request.encode_to_vec());
    hasher.finalize().to_vec()
}

fn initialize_schema(connection: &Connection) -> Result<(), CleanStoreError> {
    let tables = existing_tables(connection)?;
    let has_marker = tables.iter().any(|table| table == "clean_schema");
    if !tables.is_empty() && !has_marker {
        return Err(CleanStoreError::RetiredPilotSchema);
    }
    if has_marker {
        let version: i64 = connection
            .query_row(
                "SELECT version FROM clean_schema WHERE component='control_plane'",
                [],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        if version != SCHEMA_VERSION {
            return Err(CleanStoreError::UnsupportedSchemaVersion(version));
        }
        if tables
            != CLEAN_TABLES
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        {
            return Err(CleanStoreError::RetiredPilotSchema);
        }
        return Ok(());
    }
    connection
        .execute_batch(CLEAN_SCHEMA)
        .map_err(CleanStoreError::Database)?;
    Ok(())
}

fn existing_tables(connection: &Connection) -> Result<Vec<String>, CleanStoreError> {
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

const CLEAN_SCHEMA: &str = r"
BEGIN IMMEDIATE;
CREATE TABLE clean_schema(
    component TEXT PRIMARY KEY CHECK(component='control_plane'),
    version INTEGER NOT NULL CHECK(version > 0),
    created_at INTEGER NOT NULL
);
INSERT INTO clean_schema(component,version,created_at)
VALUES ('control_plane',1,unixepoch());

CREATE TABLE repository_connections(
    id TEXT PRIMARY KEY, status INTEGER NOT NULL, provider_key TEXT NOT NULL UNIQUE,
    payload BLOB NOT NULL, resource_version INTEGER NOT NULL CHECK(resource_version > 0),
    event_cursor INTEGER NOT NULL UNIQUE
);
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
CREATE TABLE automation_installations(
    id TEXT PRIMARY KEY,
    runtime_profile_id TEXT NOT NULL REFERENCES runtime_profiles(id),
    state INTEGER NOT NULL, version TEXT NOT NULL, payload BLOB NOT NULL,
    resource_version INTEGER NOT NULL CHECK(resource_version > 0), event_cursor INTEGER NOT NULL UNIQUE
);
CREATE TABLE development_tasks(
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
CREATE TABLE candidates(
    id TEXT PRIMARY KEY,
    development_task_id TEXT NOT NULL REFERENCES development_tasks(id),
    development_attempt_id TEXT NOT NULL REFERENCES development_attempts(id),
    repository_connection_id TEXT NOT NULL REFERENCES repository_connections(id),
    state INTEGER NOT NULL, payload BLOB NOT NULL,
    resource_version INTEGER NOT NULL CHECK(resource_version > 0), event_cursor INTEGER NOT NULL UNIQUE,
    UNIQUE(development_attempt_id)
);
CREATE TABLE change_requests(
    id TEXT PRIMARY KEY,
    repository_connection_id TEXT NOT NULL REFERENCES repository_connections(id),
    candidate_id TEXT NOT NULL REFERENCES candidates(id), state INTEGER NOT NULL,
    head_revision TEXT NOT NULL, payload BLOB NOT NULL,
    resource_version INTEGER NOT NULL CHECK(resource_version > 0), event_cursor INTEGER NOT NULL UNIQUE
);
CREATE TABLE assessments(
    id TEXT PRIMARY KEY,
    repository_connection_id TEXT NOT NULL REFERENCES repository_connections(id),
    change_request_id TEXT NOT NULL REFERENCES change_requests(id),
    head_revision TEXT NOT NULL, conclusion INTEGER NOT NULL, payload BLOB NOT NULL,
    resource_version INTEGER NOT NULL CHECK(resource_version > 0), event_cursor INTEGER NOT NULL UNIQUE
);
CREATE TABLE evidence_references(
    id TEXT PRIMARY KEY,
    assessment_id TEXT NOT NULL REFERENCES assessments(id), payload BLOB NOT NULL,
    resource_version INTEGER NOT NULL CHECK(resource_version > 0), event_cursor INTEGER NOT NULL UNIQUE
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

fn unix_seconds() -> Result<i64, CleanStoreError> {
    unix_timestamp().map(|value| value.seconds)
}

fn unix_timestamp() -> Result<Timestamp, CleanStoreError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CleanStoreError::Clock)?;
    Ok(Timestamp {
        seconds: i64::try_from(duration.as_secs()).map_err(|_| CleanStoreError::Clock)?,
        nanos: i32::try_from(duration.subsec_nanos()).map_err(|_| CleanStoreError::Clock)?,
    })
}

fn add_duration(
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

const fn timestamp_at_or_before(value: &Timestamp, other: &Timestamp) -> bool {
    value.seconds < other.seconds || (value.seconds == other.seconds && value.nanos <= other.nanos)
}

const fn timestamp(seconds: i64) -> Timestamp {
    Timestamp { seconds, nanos: 0 }
}

fn resource_id(prefix: &str) -> String {
    format!("{prefix}:{}", Uuid::now_v7())
}

fn decode<M: Message + Default>(bytes: &[u8]) -> Result<M, CleanStoreError> {
    M::decode(bytes).map_err(|_| CleanStoreError::CorruptState)
}

fn safe_metadata(values: &[v1::VerifiedMetadata], max_count: usize, max_bytes: usize) -> bool {
    values.len() <= max_count
        && values
            .iter()
            .map(|value| value.key.len().saturating_add(value.value.len()))
            .sum::<usize>()
            <= max_bytes
        && values.iter().enumerate().all(|(index, value)| {
            !value.key.is_empty()
                && value.key.len() <= 64
                && value.value.len() <= 512
                && value.key.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'-' | b'.')
                })
                && !values[..index].iter().any(|other| other.key == value.key)
        })
}

fn valid_git_revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_candidate_report(candidate: &v1::Candidate) -> bool {
    let identity_valid = candidate
        .identity
        .as_ref()
        .and_then(|identity| identity.identity.as_ref())
        .is_some_and(|identity| match identity {
            v1::candidate_identity::Identity::Git(value) => valid_git_revision(&value.commit_sha),
            v1::candidate_identity::Identity::Bundle(value) => {
                valid_digest(&value.digest) && value.size_bytes <= MAX_EVIDENCE_SIZE_BYTES
            }
        });
    candidate.state != v1::CandidateState::Unspecified as i32
        && v1::CandidateState::try_from(candidate.state).is_ok()
        && candidate.changed_paths.len() <= 256
        && candidate
            .changed_paths
            .iter()
            .map(String::len)
            .sum::<usize>()
            <= 32_768
        && candidate.changed_paths.iter().all(|path| {
            !path.is_empty()
                && path.len() <= 4_096
                && !path.starts_with('/')
                && !path.split('/').any(|part| part.is_empty() || part == "..")
        })
        && valid_git_revision(&candidate.base_revision)
        && identity_valid
}

fn valid_artifact_report(artifact: &v1::RuntimeArtifact) -> bool {
    let platform_valid = artifact.platform.as_ref().is_some_and(|platform| {
        !platform.operating_system.is_empty()
            && platform.operating_system.len() <= 32
            && !platform.architecture.is_empty()
            && platform.architecture.len() <= 32
            && platform
                .variant
                .as_ref()
                .is_none_or(|value| value.len() <= 32)
    });
    artifact.status != v1::RuntimeArtifactStatus::Unspecified as i32
        && v1::RuntimeArtifactStatus::try_from(artifact.status).is_ok()
        && !artifact.oci_repository.is_empty()
        && artifact.oci_repository.len() <= 512
        && valid_digest(&artifact.resolved_digest)
        && platform_valid
        && safe_metadata(&artifact.verified_metadata, 32, 8_192)
}

fn valid_assessment_report(assessment: &v1::Assessment) -> bool {
    let producer_valid = assessment.producer.as_ref().is_some_and(|producer| {
        producer.kind != v1::AssessmentProducerKind::Unspecified as i32
            && v1::AssessmentProducerKind::try_from(producer.kind).is_ok()
            && !producer.name.is_empty()
            && producer.name.len() <= 128
            && !producer.version.is_empty()
            && producer.version.len() <= 128
    });
    assessment.conclusion != v1::AssessmentConclusion::Unspecified as i32
        && v1::AssessmentConclusion::try_from(assessment.conclusion).is_ok()
        && assessment.evidence_reference_ids.len() <= 32
        && assessment
            .evidence_reference_ids
            .iter()
            .all(|value| !value.is_empty() && value.len() <= 512)
        && valid_git_revision(&assessment.head_revision)
        && producer_valid
        && assessment.workflow.is_some()
        && assessment.details.is_some()
}

fn valid_evidence_report(evidence: &v1::EvidenceReference) -> bool {
    !evidence.name.is_empty()
        && evidence.name.len() <= 128
        && !evidence.name.starts_with('/')
        && !evidence
            .name
            .split('/')
            .any(|part| part.is_empty() || part == "..")
        && valid_digest(&evidence.digest)
        && !evidence.media_type.is_empty()
        && evidence.media_type.len() <= 128
        && evidence.size_bytes <= MAX_EVIDENCE_SIZE_BYTES
        && safe_metadata(&evidence.metadata, 16, 4_096)
}

fn valid_usage_summary(summary: Option<&v1::ModelUsageSummary>) -> bool {
    summary.is_none_or(|summary| {
        summary
            .estimated_cost_currency
            .as_ref()
            .is_none_or(|value| value.len() <= 64)
            && summary
                .pricing_version
                .as_ref()
                .is_none_or(|value| value.len() <= 64)
    })
}

fn valid_model_usage(usage: Option<&v1::AttemptModelUsage>) -> bool {
    usage.is_some_and(|usage| {
        usage.collection_status != v1::ModelUsageCollectionStatus::Unspecified as i32
            && v1::ModelUsageCollectionStatus::try_from(usage.collection_status).is_ok()
            && usage.interactions.len() <= 64
            && usage.interactions.iter().all(|interaction| {
                !interaction.call_id.is_empty()
                    && interaction.call_id.len() <= 128
                    && interaction.provider.len() <= 128
                    && interaction.model.len() <= 128
                    && interaction
                        .resolved_model
                        .as_ref()
                        .is_none_or(|value| value.len() <= 128)
                    && interaction.stage != v1::ModelUsageStage::Unspecified as i32
                    && v1::ModelUsageStage::try_from(interaction.stage).is_ok()
                    && interaction.outcome != v1::ModelInteractionOutcome::Unspecified as i32
                    && v1::ModelInteractionOutcome::try_from(interaction.outcome).is_ok()
                    && interaction.estimated_cost.as_ref().is_none_or(|cost| {
                        cost.label.len() <= 64
                            && cost.currency.len() <= 64
                            && cost.pricing_version.len() <= 64
                    })
            })
            && usage.by_stage.len() <= 2
            && usage.by_stage.iter().enumerate().all(|(index, stage)| {
                stage.stage != v1::ModelUsageStage::Unspecified as i32
                    && v1::ModelUsageStage::try_from(stage.stage).is_ok()
                    && valid_usage_summary(stage.summary.as_ref())
                    && !usage.by_stage[..index]
                        .iter()
                        .any(|other| other.stage == stage.stage)
            })
            && valid_usage_summary(usage.total.as_ref())
    })
}

fn same_candidate_report(existing: &v1::Candidate, incoming: &v1::Candidate) -> bool {
    existing.development_attempt_id == incoming.development_attempt_id
        && existing.repository_connection_id == incoming.repository_connection_id
        && existing.base_revision == incoming.base_revision
        && existing.identity == incoming.identity
        && existing.state == incoming.state
        && existing.changed_paths == incoming.changed_paths
        && equivalent_failure(existing.failure.as_ref(), incoming.failure.as_ref())
}

fn same_artifact_report(existing: &v1::RuntimeArtifact, incoming: &v1::RuntimeArtifact) -> bool {
    existing.runtime_profile_revision_id == incoming.runtime_profile_revision_id
        && existing.oci_repository == incoming.oci_repository
        && existing.discovery_tag == incoming.discovery_tag
        && existing.resolved_digest == incoming.resolved_digest
        && existing.platform == incoming.platform
        && existing.origin == incoming.origin
        && existing.status == incoming.status
        && existing.verified_metadata == incoming.verified_metadata
        && equivalent_failure(existing.failure.as_ref(), incoming.failure.as_ref())
}

fn equivalent_failure(
    existing: Option<&v1::ResourceFailure>,
    incoming: Option<&v1::ResourceFailure>,
) -> bool {
    match (existing, incoming) {
        (None, None) => true,
        (Some(existing), Some(incoming)) => {
            existing.reason == incoming.reason
                && existing.message == incoming.message
                && existing.remediation == incoming.remediation
        }
        _ => false,
    }
}

fn bounded_failure(mut failure: Option<v1::ResourceFailure>) -> Option<v1::ResourceFailure> {
    if let Some(value) = failure.as_mut() {
        value.message = truncate_utf8(&value.message, 512);
        value.remediation = value
            .remediation
            .take()
            .map(|text| truncate_utf8(&text, 1_024));
        if value.observed_at.is_none() {
            value.observed_at = unix_seconds().ok().map(timestamp);
        }
    }
    failure
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost_types::Duration;
    use tempfile::tempdir;

    const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SECOND_REVISION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SECOND_DIGEST: &str =
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn repository_request(key: &str, repository_id: &str) -> v1::CreateRepositoryConnectionRequest {
        v1::CreateRepositoryConnectionRequest {
            idempotency_key: key.to_owned(),
            provider: Some(v1::RepositoryProvider {
                provider: Some(v1::repository_provider::Provider::Github(
                    v1::GithubRepository {
                        repository_id: repository_id.to_owned(),
                        owner: "ayni-oss".to_owned(),
                        name: "sandbox".to_owned(),
                        installation_id: 42,
                        html_url: "https://github.com/ayni-oss/sandbox".to_owned(),
                    },
                )),
            }),
            display_name: "Sandbox".to_owned(),
            default_branch: "main".to_owned(),
        }
    }

    fn profile_request(
        key: &str,
        repository_id: &str,
        snapshot_id: &str,
    ) -> v1::CreateRuntimeProfileRequest {
        v1::CreateRuntimeProfileRequest {
            idempotency_key: key.to_owned(),
            repository_connection_id: repository_id.to_owned(),
            repository_snapshot_id: snapshot_id.to_owned(),
            configuration: Some(v1::RuntimeProfileConfiguration {
                configuration: Some(
                    v1::runtime_profile_configuration::Configuration::AyniManaged(
                        v1::AyniManagedProfileRevision {
                            lock_fingerprint: "c".repeat(64),
                            ayni_version: "0.4.0".to_owned(),
                            workflow_template_version: "v1".to_owned(),
                            oci_repository: "ghcr.io/ayni-oss/sandbox/environment".to_owned(),
                        },
                    ),
                ),
            }),
        }
    }

    fn verified_artifact(revision_id: &str, digest: &str) -> v1::RuntimeArtifact {
        v1::RuntimeArtifact {
            id: String::new(),
            runtime_profile_revision_id: revision_id.to_owned(),
            oci_repository: "ghcr.io/ayni-oss/sandbox/environment".to_owned(),
            discovery_tag: Some(format!("lock-{}", "c".repeat(64))),
            resolved_digest: digest.to_owned(),
            platform: Some(v1::OciPlatform {
                operating_system: "linux".to_owned(),
                architecture: "amd64".to_owned(),
                variant: None,
            }),
            origin: Some(v1::RuntimeArtifactOrigin {
                origin: Some(v1::runtime_artifact_origin::Origin::External(
                    v1::ExternalArtifactOrigin {
                        supplied_reference: "ghcr.io/ayni-oss/sandbox/environment:discovery"
                            .to_owned(),
                        observed_at: Some(timestamp(1)),
                    },
                )),
            }),
            status: v1::RuntimeArtifactStatus::Verified as i32,
            verified_metadata: Vec::new(),
            failure: None,
            created_at: None,
            verified_at: None,
            resource_version: 0,
        }
    }

    fn ready_profile(
        store: &mut CleanStore,
    ) -> Result<
        (
            v1::RepositoryConnection,
            v1::RuntimeProfile,
            v1::RuntimeArtifact,
        ),
        CleanStoreError,
    > {
        let repository =
            store.create_repository_connection("repo-scope", &repository_request("repo", "123"))?;
        let snapshot = store.observe_repository_snapshot(&repository.id, REVISION)?;
        let (profile, revision) = store.create_runtime_profile(
            "profile-scope",
            &profile_request("profile", &repository.id, &snapshot.id),
        )?;
        let artifact =
            store.record_runtime_artifact(&profile.id, verified_artifact(&revision.id, DIGEST))?;
        Ok((repository, store.runtime_profile(&profile.id)?, artifact))
    }

    fn task_request(
        repository_id: &str,
        profile_id: &str,
        key: &str,
    ) -> v1::CreateDevelopmentTaskRequest {
        v1::CreateDevelopmentTaskRequest {
            idempotency_key: key.to_owned(),
            request: Some(v1::DevelopmentTaskRequest {
                repository_connection_id: repository_id.to_owned(),
                repository_revision: REVISION.to_owned(),
                runtime_profile_id: profile_id.to_owned(),
                agent: Some(v1::AgentSelection {
                    provider: "openai".to_owned(),
                    agent: "coding-agent".to_owned(),
                    model: "gpt-5".to_owned(),
                }),
                request: "Make the bounded change".to_owned(),
                acceptance_criteria: vec!["Tests pass".to_owned()],
                change_constraints: Some(v1::ChangeConstraints::default()),
                deadline: Some(Duration {
                    seconds: 3_600,
                    nanos: 0,
                }),
            }),
        }
    }

    #[test]
    fn initializes_exact_clean_schema_and_reopens() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("orchestrator.sqlite");
        let store = CleanStore::open(&path, 3)?;
        assert_eq!(store.schema_version()?, 1);
        assert!(existing_tables(&store.connection)?.contains(&"runtime_profiles".to_owned()));
        assert!(!existing_tables(&store.connection)?.contains(&"repository_bindings".to_owned()));
        drop(store);
        let reopened = CleanStore::open(path, 3)?;
        reopened.integrity_check()?;
        Ok(())
    }

    #[test]
    fn rejects_retired_pilot_schema_without_mutating_it() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempdir()?;
        let path = directory.path().join("orchestrator.sqlite");
        let connection = Connection::open(&path)?;
        connection.execute_batch("CREATE TABLE tasks(id TEXT PRIMARY KEY);")?;
        drop(connection);
        assert!(matches!(
            CleanStore::open(&path, 3),
            Err(CleanStoreError::RetiredPilotSchema)
        ));
        let connection = Connection::open(path)?;
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tasks'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn guarded_reset_of_retired_volume_produces_exact_clean_baseline()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("orchestrator.sqlite");
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE tasks(id TEXT PRIMARY KEY); CREATE TABLE preparation_records(id TEXT);",
        )?;
        drop(connection);
        assert!(matches!(
            CleanStore::open(&path, 3),
            Err(CleanStoreError::RetiredPilotSchema)
        ));

        fs::remove_file(&path)?;
        let store = CleanStore::open(path, 3)?;
        assert_eq!(
            existing_tables(&store.connection)?,
            [
                "assessments",
                "automation_installations",
                "candidates",
                "change_requests",
                "clean_schema",
                "development_attempts",
                "development_tasks",
                "durable_actions",
                "event_watermarks",
                "evidence_references",
                "page_snapshots",
                "reconciliation_cursors",
                "repository_connections",
                "repository_snapshots",
                "resource_events",
                "runtime_artifacts",
                "runtime_profile_revisions",
                "runtime_profiles",
            ]
            .map(str::to_owned)
        );
        Ok(())
    }

    #[test]
    fn idempotent_create_replays_and_conflicting_content_fails() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let request = repository_request("public-key", "123");
        let first = store.create_repository_connection("scope", &request)?;
        let replay = store.create_repository_connection("scope", &request)?;
        assert_eq!(replay.id, first.id);
        let changed = repository_request("public-key", "456");
        assert!(matches!(
            store.create_repository_connection("scope", &changed),
            Err(CleanStoreError::IdempotencyConflict)
        ));
        Ok(())
    }

    #[test]
    fn pages_retain_values_across_writes_and_restart() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("orchestrator.sqlite");
        let mut store = CleanStore::open(&path, 3)?;
        let first = store.create_repository_connection("one", &repository_request("one", "1"))?;
        let second = store.create_repository_connection("two", &repository_request("two", "2"))?;
        let page = store.list_repository_connections(
            None,
            Some(&v1::PageRequest {
                page_size: 1,
                page_token: String::new(),
            }),
        )?;
        assert_eq!(page.items.len(), 1);
        assert!(!page.next_page_token.is_empty());
        let expected_remaining = if page.items[0].id == first.id {
            second.id
        } else {
            first.id
        };
        let token = page.next_page_token;
        let mut tampered = token.clone().into_bytes();
        let last = tampered
            .last_mut()
            .ok_or(CleanStoreError::InvalidPageToken)?;
        *last = if *last == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(tampered)?;
        assert!(matches!(
            store.list_repository_connections(
                None,
                Some(&v1::PageRequest {
                    page_size: 1,
                    page_token: tampered,
                }),
            ),
            Err(CleanStoreError::InvalidPageToken)
        ));
        store.create_repository_connection("three", &repository_request("three", "3"))?;
        drop(store);
        let mut reopened = CleanStore::open(path, 3)?;
        let next = reopened.list_repository_connections(
            None,
            Some(&v1::PageRequest {
                page_size: 1,
                page_token: token,
            }),
        )?;
        assert_eq!(next.items[0].id, expected_remaining);
        assert!(next.next_page_token.is_empty());
        Ok(())
    }

    #[test]
    fn pagination_rejects_unbounded_snapshots_and_responses() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let oversized = vec![vec![0_u8; MAX_PAGE_SNAPSHOT_BYTES.saturating_add(1)]];
        assert!(matches!(
            store.materialize_page::<v1::RepositoryConnection>(
                "ListRepositoryConnections",
                &encode_filter(&[""]),
                0,
                oversized,
                0,
                1,
            ),
            Err(CleanStoreError::BoundExceeded)
        ));
        store
            .connection
            .execute_batch(
                "CREATE TEMP TABLE oversized_payloads(payload BLOB NOT NULL);
             INSERT INTO oversized_payloads VALUES (zeroblob(5000000)),(zeroblob(5000000));",
            )
            .map_err(CleanStoreError::Database)?;
        let mut statement = store
            .connection
            .prepare("SELECT payload FROM oversized_payloads")
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        assert!(matches!(
            collect_bounded_payloads(&mut rows),
            Err(CleanStoreError::BoundExceeded)
        ));
        Ok(())
    }

    #[test]
    fn external_action_recovers_at_every_durable_boundary() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempdir()?;
        let path = directory.path().join("orchestrator.sqlite");
        let request = v1::ReconcileRuntimeProfileRequest {
            idempotency_key: "reconcile".to_owned(),
            runtime_profile_id: "profile:missing".to_owned(),
        };

        let mut store = CleanStore::open(&path, 3)?;
        let pending = store.begin_external_action(
            "ReconcileRuntimeProfile",
            "scoped-key",
            &request,
            "runtime_profile",
            "profile:missing",
        )?;
        assert_eq!(pending.state, DurableActionState::Pending);
        drop(store);

        let mut store = CleanStore::open(&path, 3)?;
        let recovered = store.begin_external_action(
            "ReconcileRuntimeProfile",
            "scoped-key",
            &request,
            "runtime_profile",
            "profile:missing",
        )?;
        assert_eq!(recovered.state, DurableActionState::Pending);
        assert_eq!(
            store.connection.query_row(
                "SELECT COUNT(*) FROM durable_actions WHERE operation='ReconcileRuntimeProfile'",
                [],
                |row| row.get::<_, i64>(0),
            )?,
            1
        );
        store.fail_external_action("ReconcileRuntimeProfile", "scoped-key", &"é".repeat(600))?;
        drop(store);

        let mut store = CleanStore::open(&path, 3)?;
        assert_eq!(
            store
                .durable_action("ReconcileRuntimeProfile", "scoped-key")?
                .map(|value| value.state),
            Some(DurableActionState::Failed)
        );
        assert!(matches!(
            store.complete_external_action("ReconcileRuntimeProfile", "scoped-key"),
            Err(CleanStoreError::InvalidTransition)
        ));
        store.requeue_external_action("ReconcileRuntimeProfile", "scoped-key")?;
        drop(store);

        let mut store = CleanStore::open(&path, 3)?;
        assert_eq!(
            store
                .pending_external_actions("ReconcileRuntimeProfile")?
                .len(),
            1
        );
        store.complete_external_action("ReconcileRuntimeProfile", "scoped-key")?;
        drop(store);

        let mut store = CleanStore::open(path, 3)?;
        let completed = store.begin_external_action(
            "ReconcileRuntimeProfile",
            "scoped-key",
            &request,
            "runtime_profile",
            "profile:missing",
        )?;
        assert_eq!(completed.state, DurableActionState::Completed);
        assert_eq!(
            v1::ReconcileRuntimeProfileRequest::decode(completed.request_payload.as_slice(),)?,
            request
        );
        let conflicting = v1::ReconcileRuntimeProfileRequest {
            runtime_profile_id: "profile:different".to_owned(),
            ..request
        };
        assert!(matches!(
            store.begin_external_action(
                "ReconcileRuntimeProfile",
                "scoped-key",
                &conflicting,
                "runtime_profile",
                "profile:different",
            ),
            Err(CleanStoreError::IdempotencyConflict)
        ));
        Ok(())
    }

    #[test]
    fn duplicate_and_late_artifact_reports_cannot_roll_back_active_revision()
    -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let (repository, profile, first_artifact) = ready_profile(&mut store)?;
        let duplicate = store.record_runtime_artifact(
            &profile.id,
            verified_artifact(
                &profile
                    .active_revision_id
                    .clone()
                    .ok_or(CleanStoreError::CorruptState)?,
                DIGEST,
            ),
        )?;
        assert_eq!(duplicate.id, first_artifact.id);
        let competing = store.record_runtime_artifact(
            &profile.id,
            verified_artifact(
                &profile
                    .active_revision_id
                    .clone()
                    .ok_or(CleanStoreError::CorruptState)?,
                "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            ),
        )?;
        assert_ne!(competing.id, first_artifact.id);
        assert_eq!(
            store
                .runtime_profile(&profile.id)?
                .active_runtime_artifact_id,
            Some(first_artifact.id)
        );

        let first_revision_id = profile
            .active_revision_id
            .clone()
            .ok_or(CleanStoreError::CorruptState)?;
        let configuration = store
            .runtime_profile_revision(&first_revision_id)?
            .configuration
            .ok_or(CleanStoreError::CorruptState)?;
        let snapshot = store.observe_repository_snapshot(&repository.id, SECOND_REVISION)?;
        let second_revision =
            store.record_runtime_profile_revision(&profile.id, &snapshot.id, configuration)?;
        let late_digest = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        store.record_runtime_artifact(
            &profile.id,
            verified_artifact(&first_revision_id, late_digest),
        )?;
        let retained = store.runtime_profile(&profile.id)?;
        assert_eq!(retained.active_revision_id, Some(second_revision.id));
        assert_eq!(retained.state, v1::RuntimeProfileState::Updating as i32);
        assert!(retained.active_runtime_artifact_id.is_none());
        Ok(())
    }

    #[test]
    fn staged_artifact_verification_and_loss_update_profile_binding() -> Result<(), CleanStoreError>
    {
        let mut store = CleanStore::in_memory()?;
        let repository =
            store.create_repository_connection("repo-scope", &repository_request("repo", "123"))?;
        let snapshot = store.observe_repository_snapshot(&repository.id, REVISION)?;
        let (profile, revision) = store.create_runtime_profile(
            "profile-scope",
            &profile_request("profile", &repository.id, &snapshot.id),
        )?;
        let mut report = verified_artifact(&revision.id, DIGEST);
        report.status = v1::RuntimeArtifactStatus::Discovered as i32;
        report.verified_at = None;
        let discovered = store.record_runtime_artifact(&profile.id, report)?;
        let verifying = store.transition_runtime_artifact(
            &discovered.id,
            discovered.resource_version,
            v1::RuntimeArtifactStatus::Verifying,
            None,
        )?;
        let verified = store.transition_runtime_artifact(
            &verifying.id,
            verifying.resource_version,
            v1::RuntimeArtifactStatus::Verified,
            None,
        )?;
        let ready = store.runtime_profile(&profile.id)?;
        assert_eq!(ready.active_runtime_artifact_id, Some(verified.id.clone()));
        assert_eq!(ready.state, v1::RuntimeProfileState::Ready as i32);
        let unavailable = store.transition_runtime_artifact(
            &verified.id,
            verified.resource_version,
            v1::RuntimeArtifactStatus::Unavailable,
            Some(v1::ResourceFailure {
                reason: v1::ErrorReason::RuntimeArtifactNotVerified as i32,
                message: "artifact is no longer available".to_owned(),
                remediation: None,
                observed_at: None,
            }),
        )?;
        assert_eq!(
            unavailable.status,
            v1::RuntimeArtifactStatus::Unavailable as i32
        );
        let degraded = store.runtime_profile(&profile.id)?;
        assert!(degraded.active_runtime_artifact_id.is_none());
        assert_eq!(degraded.state, v1::RuntimeProfileState::Degraded as i32);
        assert!(degraded.failure.is_some());

        let replacement = store
            .record_runtime_artifact(&profile.id, verified_artifact(&revision.id, SECOND_DIGEST))?;
        let restored = store.runtime_profile(&profile.id)?;
        assert_eq!(restored.active_runtime_artifact_id, Some(replacement.id));
        assert_eq!(restored.state, v1::RuntimeProfileState::Ready as i32);
        assert!(restored.failure.is_none());
        Ok(())
    }

    #[test]
    fn task_freezes_runtime_and_rejects_stale_or_invalid_transitions() -> Result<(), CleanStoreError>
    {
        let mut store = CleanStore::in_memory()?;
        let (repository, profile, first_artifact) = ready_profile(&mut store)?;
        let (task, attempt) = store.create_development_task(
            "task-scope",
            &task_request(&repository.id, &profile.id, "task"),
        )?;
        let frozen = task
            .snapshot
            .as_ref()
            .and_then(|value| value.runtime.as_ref())
            .ok_or(CleanStoreError::CorruptState)?;
        assert_eq!(frozen.runtime_artifact_id, first_artifact.id);
        let revision_id = profile
            .active_revision_id
            .clone()
            .ok_or(CleanStoreError::CorruptState)?;
        let configuration = store
            .runtime_profile_revision(&revision_id)?
            .configuration
            .ok_or(CleanStoreError::CorruptState)?;
        let second_snapshot = store.observe_repository_snapshot(&repository.id, SECOND_REVISION)?;
        let second_revision = store.record_runtime_profile_revision(
            &profile.id,
            &second_snapshot.id,
            configuration,
        )?;
        let mut waiting_request = task_request(&repository.id, &profile.id, "waiting-task");
        if let Some(request) = waiting_request.request.as_mut() {
            request.repository_revision = SECOND_REVISION.to_owned();
        }
        assert!(matches!(
            store.create_development_task("waiting-task-scope", &waiting_request),
            Err(CleanStoreError::ProfileNotReady)
        ));
        let second = store.record_runtime_artifact(
            &profile.id,
            verified_artifact(&second_revision.id, SECOND_DIGEST),
        )?;
        assert_ne!(second.id, first_artifact.id);
        let (new_task, _) = store.create_development_task("new-task-scope", &waiting_request)?;
        assert_eq!(
            new_task
                .snapshot
                .and_then(|value| value.runtime)
                .map(|value| value.runtime_artifact_id),
            Some(second.id)
        );
        let retained = store.development_task(&task.id)?;
        assert_eq!(
            retained
                .snapshot
                .and_then(|value| value.runtime)
                .map(|value| value.runtime_artifact_id),
            Some(first_artifact.id)
        );
        assert!(matches!(
            store.transition_attempt(
                &attempt.id,
                attempt.resource_version,
                v1::DevelopmentAttemptState::Succeeded,
                None,
            ),
            Err(CleanStoreError::InvalidTransition)
        ));
        let running = store.transition_attempt(
            &attempt.id,
            attempt.resource_version,
            v1::DevelopmentAttemptState::Running,
            None,
        )?;
        let duplicate = store.transition_attempt(
            &attempt.id,
            attempt.resource_version,
            v1::DevelopmentAttemptState::Running,
            None,
        )?;
        assert_eq!(duplicate.resource_version, running.resource_version);
        assert!(matches!(
            store.transition_attempt(
                &attempt.id,
                attempt.resource_version,
                v1::DevelopmentAttemptState::Failed,
                None,
            ),
            Err(CleanStoreError::StaleVersion)
        ));
        let succeeded = store.transition_attempt(
            &attempt.id,
            running.resource_version,
            v1::DevelopmentAttemptState::Succeeded,
            None,
        )?;
        assert_eq!(
            succeeded.state,
            v1::DevelopmentAttemptState::Succeeded as i32
        );
        assert!(matches!(
            store.transition_attempt(
                &attempt.id,
                succeeded.resource_version,
                v1::DevelopmentAttemptState::Failed,
                None,
            ),
            Err(CleanStoreError::InvalidTransition)
        ));
        Ok(())
    }

    #[test]
    fn cancellation_is_terminal_against_late_reports() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let (repository, profile, _) = ready_profile(&mut store)?;
        let (task, attempt) = store.create_development_task(
            "task-scope",
            &task_request(&repository.id, &profile.id, "task"),
        )?;
        let cancelled = store.cancel_development_task(
            "cancel-scope",
            &v1::CancelDevelopmentTaskRequest {
                idempotency_key: "cancel".to_owned(),
                development_task_id: task.id.clone(),
            },
        )?;
        assert_eq!(cancelled.state, v1::DevelopmentTaskState::Cancelled as i32);
        store.cancel_development_task(
            "second-cancel-scope",
            &v1::CancelDevelopmentTaskRequest {
                idempotency_key: "second-cancel".to_owned(),
                development_task_id: task.id,
            },
        )?;
        let (second_task, _) = store.create_development_task(
            "second-task-scope",
            &task_request(&repository.id, &profile.id, "second-task"),
        )?;
        assert!(matches!(
            store.cancel_development_task(
                "second-cancel-scope",
                &v1::CancelDevelopmentTaskRequest {
                    idempotency_key: "second-cancel".to_owned(),
                    development_task_id: second_task.id,
                },
            ),
            Err(CleanStoreError::IdempotencyConflict)
        ));
        let cancelled_attempt = store.development_attempt(&attempt.id)?;
        assert!(matches!(
            store.transition_attempt(
                &attempt.id,
                cancelled_attempt.resource_version,
                v1::DevelopmentAttemptState::Succeeded,
                None,
            ),
            Err(CleanStoreError::InvalidTransition)
        ));
        Ok(())
    }

    #[test]
    fn clean_marker_does_not_hide_retired_tables() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("orchestrator.sqlite");
        drop(CleanStore::open(&path, 3)?);
        let connection = Connection::open(&path)?;
        connection.execute_batch("CREATE TABLE tasks(id TEXT PRIMARY KEY);")?;
        drop(connection);
        assert!(matches!(
            CleanStore::open(&path, 3),
            Err(CleanStoreError::RetiredPilotSchema)
        ));
        let connection = Connection::open(path)?;
        assert_eq!(
            connection.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tasks'",
                [],
                |row| row.get::<_, i64>(0),
            )?,
            1
        );
        Ok(())
    }

    #[test]
    fn create_profile_replay_keeps_the_original_revision() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let repository =
            store.create_repository_connection("repo-scope", &repository_request("repo", "123"))?;
        let snapshot = store.observe_repository_snapshot(&repository.id, REVISION)?;
        let request = profile_request("profile", &repository.id, &snapshot.id);
        let (profile, initial_revision) =
            store.create_runtime_profile("profile-scope", &request)?;
        let configuration = initial_revision
            .configuration
            .clone()
            .ok_or(CleanStoreError::CorruptState)?;
        let next_snapshot = store.observe_repository_snapshot(&repository.id, SECOND_REVISION)?;
        let next_revision = store.record_runtime_profile_revision(
            &profile.id,
            &next_snapshot.id,
            configuration.clone(),
        )?;
        let stale_duplicate =
            store.record_runtime_profile_revision(&profile.id, &snapshot.id, configuration)?;
        assert_eq!(stale_duplicate.id, initial_revision.id);
        assert_eq!(
            store.runtime_profile(&profile.id)?.active_revision_id,
            Some(next_revision.id.clone())
        );
        let (replayed_profile, replayed_revision) =
            store.create_runtime_profile("profile-scope", &request)?;
        assert_eq!(replayed_revision.id, initial_revision.id);
        assert_eq!(replayed_revision.revision_number, 1);
        assert_eq!(replayed_profile.active_revision_id, Some(next_revision.id));
        Ok(())
    }

    #[test]
    fn create_task_replay_keeps_the_original_initial_attempt() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let (repository, profile, _) = ready_profile(&mut store)?;
        let request = task_request(&repository.id, &profile.id, "task");
        let (task, initial) = store.create_development_task("task-scope", &request)?;
        let running = store.transition_attempt(
            &initial.id,
            initial.resource_version,
            v1::DevelopmentAttemptState::Running,
            None,
        )?;
        store.transition_attempt(
            &initial.id,
            running.resource_version,
            v1::DevelopmentAttemptState::Failed,
            None,
        )?;
        let retry = store.retry_development_task(
            "retry-scope",
            &v1::RetryDevelopmentTaskRequest {
                idempotency_key: "retry".to_owned(),
                development_task_id: task.id,
            },
        )?;
        assert_eq!(retry.attempt_number, 2);
        let (replayed_task, replayed_initial) =
            store.create_development_task("task-scope", &request)?;
        assert_eq!(replayed_initial.id, initial.id);
        assert_eq!(replayed_initial.attempt_number, 1);
        assert_eq!(replayed_task.active_attempt_id, Some(retry.id));
        Ok(())
    }

    #[test]
    fn semantically_empty_or_index_mismatched_payload_fails_closed()
    -> Result<(), Box<dyn std::error::Error>> {
        for corruption in ["payload=x''", "status=999"] {
            let directory = tempdir()?;
            let path = directory.path().join("orchestrator.sqlite");
            let mut store = CleanStore::open(&path, 3)?;
            let repository = store
                .create_repository_connection("repo-scope", &repository_request("repo", "123"))?;
            store.connection.execute(
                &format!("UPDATE repository_connections SET {corruption} WHERE id=?1"),
                [&repository.id],
            )?;
            drop(store);
            assert!(matches!(
                CleanStore::open(path, 3),
                Err(CleanStoreError::CorruptState)
            ));
        }
        Ok(())
    }

    #[test]
    fn corrupt_payload_fails_closed_on_read_and_restart() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempdir()?;
        let path = directory.path().join("orchestrator.sqlite");
        let mut store = CleanStore::open(&path, 3)?;
        let repository =
            store.create_repository_connection("repo-scope", &repository_request("repo", "123"))?;
        store.connection.execute(
            "UPDATE repository_connections SET payload=x'00' WHERE id=?1",
            [&repository.id],
        )?;
        assert!(matches!(
            store.repository_connection(&repository.id),
            Err(CleanStoreError::CorruptState)
        ));
        drop(store);
        assert!(matches!(
            CleanStore::open(path, 3),
            Err(CleanStoreError::CorruptState)
        ));
        Ok(())
    }

    #[test]
    fn corrupt_event_or_page_snapshot_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        for corruption in ["event", "page"] {
            let directory = tempdir()?;
            let path = directory.path().join("orchestrator.sqlite");
            let mut store = CleanStore::open(&path, 3)?;
            store.create_repository_connection("one", &repository_request("one", "1"))?;
            if corruption == "event" {
                store.connection.execute(
                    "UPDATE resource_events SET payload=x'' WHERE cursor=(SELECT MIN(cursor) FROM resource_events)",
                    [],
                )?;
            } else {
                store.create_repository_connection("two", &repository_request("two", "2"))?;
                let page = store.list_repository_connections(
                    None,
                    Some(&v1::PageRequest {
                        page_size: 1,
                        page_token: String::new(),
                    }),
                )?;
                assert!(!page.next_page_token.is_empty());
                store
                    .connection
                    .execute("UPDATE page_snapshots SET items='[\"not-base64!\"]'", [])?;
            }
            drop(store);
            assert!(matches!(
                CleanStore::open(path, 3),
                Err(CleanStoreError::CorruptState)
            ));
        }
        Ok(())
    }

    #[test]
    fn publication_action_retains_intent_and_requeues_after_observation()
    -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let (repository, profile, _) = ready_profile(&mut store)?;
        let (task, attempt) = store.create_development_task(
            "task-scope",
            &task_request(&repository.id, &profile.id, "task"),
        )?;
        let running = store.transition_attempt(
            &attempt.id,
            attempt.resource_version,
            v1::DevelopmentAttemptState::Running,
            None,
        )?;
        let succeeded = store.transition_attempt(
            &attempt.id,
            running.resource_version,
            v1::DevelopmentAttemptState::Succeeded,
            None,
        )?;
        let candidate = store.record_candidate(v1::Candidate {
            development_attempt_id: succeeded.id,
            repository_connection_id: repository.id,
            base_revision: REVISION.to_owned(),
            identity: Some(v1::CandidateIdentity {
                identity: Some(v1::candidate_identity::Identity::Git(v1::GitCandidate {
                    commit_sha: SECOND_REVISION.to_owned(),
                })),
            }),
            state: v1::CandidateState::Available as i32,
            changed_paths: vec!["src/lib.rs".to_owned()],
            ..v1::Candidate::default()
        })?;
        let mut duplicate_candidate = candidate.clone();
        duplicate_candidate.id.clear();
        duplicate_candidate.created_at = None;
        duplicate_candidate.resource_version = 0;
        assert_eq!(
            store.record_candidate(duplicate_candidate)?.id,
            candidate.id
        );
        let request = v1::PublishCandidateRequest {
            idempotency_key: "publish".to_owned(),
            candidate_id: candidate.id,
            title: "feat: bounded change".to_owned(),
            body: "Body".to_owned(),
            base_branch: "main".to_owned(),
            draft: true,
        };
        let change_request = store.publish_candidate("publish-scope", &request)?;
        let action = store
            .durable_action("PublishCandidate", "publish-scope")?
            .ok_or(CleanStoreError::CorruptState)?;
        assert_eq!(action.state, DurableActionState::Pending);
        assert_eq!(
            v1::PublishCandidateRequest::decode(action.request_payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?,
            request
        );
        let provider = v1::ChangeRequestProvider {
            provider: Some(v1::change_request_provider::Provider::Github(
                v1::GithubChangeRequest {
                    repository_id: "123".to_owned(),
                    pull_request_number: 7,
                    html_url: "https://github.com/ayni-oss/sandbox/pull/7".to_owned(),
                },
            )),
        };
        let failure = v1::ResourceFailure {
            reason: v1::ErrorReason::ProviderUnavailable as i32,
            message: "provider response was uncertain".to_owned(),
            remediation: None,
            observed_at: None,
        };
        let failed = store.reconcile_change_request(
            "publish-scope",
            change_request.resource_version,
            provider.clone(),
            v1::ChangeRequestState::Failed,
            SECOND_REVISION,
            Some(failure.clone()),
        )?;
        let duplicate = store.reconcile_change_request(
            "publish-scope",
            change_request.resource_version,
            provider,
            v1::ChangeRequestState::Failed,
            SECOND_REVISION,
            Some(failure),
        )?;
        assert_eq!(duplicate.resource_version, failed.resource_version);
        let pending =
            store.retry_change_request_publication("publish-scope", failed.resource_version)?;
        assert_eq!(pending.state, v1::ChangeRequestState::Publishing as i32);
        assert_eq!(
            store
                .durable_action("PublishCandidate", "publish-scope")?
                .map(|value| value.state),
            Some(DurableActionState::Pending)
        );
        assert_eq!(
            store.development_task(&task.id)?.state,
            v1::DevelopmentTaskState::Succeeded as i32
        );
        Ok(())
    }

    #[test]
    fn fractional_deadlines_preserve_subsecond_precision() -> Result<(), CleanStoreError> {
        let base = Timestamp {
            seconds: 10,
            nanos: 900_000_000,
        };
        let deadline = add_duration(
            &base,
            &prost_types::Duration {
                seconds: 0,
                nanos: 200_000_000,
            },
        )?;
        assert_eq!(
            deadline,
            Timestamp {
                seconds: 11,
                nanos: 100_000_000,
            }
        );
        assert!(!timestamp_at_or_before(
            &deadline,
            &Timestamp {
                seconds: 11,
                nanos: 99_999_999,
            }
        ));
        assert!(timestamp_at_or_before(&deadline, &deadline));
        Ok(())
    }

    #[test]
    fn reconciliation_expires_task_and_attempt_deadlines() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let (repository, profile, _) = ready_profile(&mut store)?;
        let (task, attempt) = store.create_development_task(
            "task-scope",
            &task_request(&repository.id, &profile.id, "task"),
        )?;
        assert_eq!(store.expire_deadlines(&timestamp(i64::MAX / 2))?, 1);
        assert_eq!(
            store.development_task(&task.id)?.state,
            v1::DevelopmentTaskState::DeadlineExceeded as i32
        );
        assert_eq!(
            store.development_attempt(&attempt.id)?.state,
            v1::DevelopmentAttemptState::DeadlineExceeded as i32
        );
        assert_eq!(store.expire_deadlines(&timestamp(i64::MAX / 2))?, 0);
        Ok(())
    }

    #[test]
    fn transition_tables_reject_every_terminal_source() {
        use v1::DevelopmentAttemptState as State;
        for terminal in [
            State::Succeeded,
            State::Failed,
            State::Cancelled,
            State::DeadlineExceeded,
            State::Unsupported,
        ] {
            for target in [
                State::Queued,
                State::Running,
                State::WaitingForInput,
                State::Succeeded,
                State::Failed,
                State::Cancelled,
                State::DeadlineExceeded,
                State::Unsupported,
            ] {
                assert!(!valid_attempt_transition(terminal, target));
            }
        }
    }

    #[test]
    fn resource_transition_tables_match_only_supported_edges() {
        use v1::{
            AutomationInstallationState as Automation, CandidateState as Candidate,
            RepositoryConnectionStatus as Repository, RuntimeArtifactStatus as Artifact,
            RuntimeProfileState as Profile,
        };
        let repository_states = [
            Repository::Connecting,
            Repository::Connected,
            Repository::Degraded,
            Repository::Revoked,
            Repository::Failed,
        ];
        let repository_edges = [
            (Repository::Connecting, Repository::Connected),
            (Repository::Connecting, Repository::Degraded),
            (Repository::Connecting, Repository::Revoked),
            (Repository::Connecting, Repository::Failed),
            (Repository::Connected, Repository::Degraded),
            (Repository::Connected, Repository::Revoked),
            (Repository::Degraded, Repository::Connecting),
            (Repository::Degraded, Repository::Connected),
            (Repository::Degraded, Repository::Revoked),
            (Repository::Degraded, Repository::Failed),
        ];
        for current in repository_states {
            for next in repository_states {
                assert_eq!(
                    valid_repository_transition(current, next),
                    repository_edges.contains(&(current, next)),
                    "unexpected repository transition {current:?} -> {next:?}"
                );
            }
        }

        let profile_states = [
            Profile::SetupRequired,
            Profile::SetupPrOpen,
            Profile::AwaitingEnvironment,
            Profile::Ready,
            Profile::Updating,
            Profile::Degraded,
            Profile::Failed,
        ];
        for current in profile_states {
            for next in profile_states {
                let expected = match current {
                    Profile::SetupRequired => matches!(
                        next,
                        Profile::SetupPrOpen
                            | Profile::AwaitingEnvironment
                            | Profile::Degraded
                            | Profile::Failed
                    ),
                    Profile::SetupPrOpen => matches!(
                        next,
                        Profile::SetupRequired
                            | Profile::AwaitingEnvironment
                            | Profile::Degraded
                            | Profile::Failed
                    ),
                    Profile::AwaitingEnvironment => matches!(
                        next,
                        Profile::Ready | Profile::Updating | Profile::Degraded | Profile::Failed
                    ),
                    Profile::Ready => matches!(next, Profile::Updating | Profile::Degraded),
                    Profile::Updating => {
                        matches!(next, Profile::Ready | Profile::Degraded | Profile::Failed)
                    }
                    Profile::Degraded => matches!(
                        next,
                        Profile::SetupRequired
                            | Profile::AwaitingEnvironment
                            | Profile::Updating
                            | Profile::Ready
                            | Profile::Failed
                    ),
                    Profile::Failed | Profile::Unspecified => false,
                };
                assert_eq!(valid_profile_transition(current, next), expected);
            }
        }

        let artifact_states = [
            Artifact::Discovered,
            Artifact::Verifying,
            Artifact::Verified,
            Artifact::Rejected,
            Artifact::Unavailable,
            Artifact::Failed,
        ];
        for current in artifact_states {
            for next in artifact_states {
                let expected = match current {
                    Artifact::Discovered => matches!(
                        next,
                        Artifact::Verifying
                            | Artifact::Rejected
                            | Artifact::Unavailable
                            | Artifact::Failed
                    ),
                    Artifact::Verifying => matches!(
                        next,
                        Artifact::Verified
                            | Artifact::Rejected
                            | Artifact::Unavailable
                            | Artifact::Failed
                    ),
                    Artifact::Verified => next == Artifact::Unavailable,
                    Artifact::Unavailable => matches!(next, Artifact::Verifying | Artifact::Failed),
                    Artifact::Rejected | Artifact::Failed | Artifact::Unspecified => false,
                };
                assert_eq!(valid_artifact_transition(current, next), expected);
            }
        }

        let automation_states = [
            Automation::SetupRequired,
            Automation::SetupPrOpen,
            Automation::Installed,
            Automation::Updating,
            Automation::Degraded,
            Automation::Failed,
        ];
        for current in automation_states {
            for next in automation_states {
                let expected = match current {
                    Automation::SetupRequired => matches!(
                        next,
                        Automation::SetupPrOpen
                            | Automation::Installed
                            | Automation::Degraded
                            | Automation::Failed
                    ),
                    Automation::SetupPrOpen => matches!(
                        next,
                        Automation::SetupRequired
                            | Automation::Installed
                            | Automation::Updating
                            | Automation::Degraded
                            | Automation::Failed
                    ),
                    Automation::Installed => {
                        matches!(next, Automation::Updating | Automation::Degraded)
                    }
                    Automation::Updating => matches!(
                        next,
                        Automation::Installed | Automation::Degraded | Automation::Failed
                    ),
                    Automation::Degraded => matches!(
                        next,
                        Automation::SetupRequired
                            | Automation::Updating
                            | Automation::Installed
                            | Automation::Failed
                    ),
                    Automation::Failed | Automation::Unspecified => false,
                };
                assert_eq!(valid_automation_transition(current, next), expected);
            }
        }
        assert!(valid_candidate_transition(
            Candidate::Available,
            Candidate::Rejected
        ));
        assert!(!valid_candidate_transition(
            Candidate::Rejected,
            Candidate::Available
        ));
        assert!(!valid_candidate_transition(
            Candidate::Available,
            Candidate::Available
        ));
    }

    #[test]
    fn watch_rejects_pruned_and_future_cursors() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let transaction = store
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        for version in 1..=MAX_EVENTS_PER_SERVICE.saturating_add(2) {
            let resource = v1::RepositoryConnection {
                id: format!("repository:{version}"),
                resource_version: u64::try_from(version)
                    .map_err(|_| CleanStoreError::CorruptState)?,
                ..v1::RepositoryConnection::default()
            };
            append_event(
                &transaction,
                REPOSITORY_SERVICE,
                "repository_connection",
                &resource.id,
                resource.resource_version,
                Some(&resource.id),
                None,
                &resource,
            )?;
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        let watermark: i64 = store
            .connection
            .query_row(
                "SELECT first_cursor FROM event_watermarks WHERE service=?1",
                [REPOSITORY_SERVICE],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        assert!(watermark > 1);
        assert!(matches!(
            store.events_after(REPOSITORY_SERVICE, 0, None, None),
            Err(CleanStoreError::WatchCursorExpired)
        ));
        let high_water = store.current_cursor(REPOSITORY_SERVICE)?;
        assert!(matches!(
            store.events_after(REPOSITORY_SERVICE, high_water.saturating_add(1), None, None,),
            Err(CleanStoreError::WatchCursorExpired)
        ));
        assert!(
            !store
                .events_after(
                    REPOSITORY_SERVICE,
                    u64::try_from(watermark.saturating_sub(1))
                        .map_err(|_| CleanStoreError::CorruptState)?,
                    None,
                    None,
                )?
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn service_events_are_monotonic_and_resumable() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let first = store.create_repository_connection("one", &repository_request("one", "1"))?;
        let events = store.events_after(REPOSITORY_SERVICE, 0, Some(&first.id), None)?;
        assert_eq!(events.len(), 1);
        let cursor = events[0].cursor;
        store.observe_repository_snapshot(&first.id, SECOND_REVISION)?;
        let resumed = store.events_after(REPOSITORY_SERVICE, cursor, Some(&first.id), None)?;
        assert_eq!(resumed.len(), 1);
        assert!(resumed[0].cursor > cursor);
        store
            .connection
            .execute(
                "DELETE FROM resource_events WHERE service=?1",
                [REPOSITORY_SERVICE],
            )
            .map_err(CleanStoreError::Database)?;
        let (current, high_water) =
            store.initial_events(REPOSITORY_SERVICE, Some(&first.id), None)?;
        assert_eq!(current.len(), 2);
        assert_eq!(high_water, 0);
        Ok(())
    }
}

fn provider_key(provider: &v1::RepositoryProvider) -> Result<String, CleanStoreError> {
    match provider.provider.as_ref() {
        Some(v1::repository_provider::Provider::Github(github)) => {
            Ok(format!("github:{}", github.repository_id))
        }
        None => Err(CleanStoreError::CorruptState),
    }
}

fn page_items_decode(method: &str, items: &[Vec<u8>]) -> bool {
    macro_rules! all_decode {
        ($message:ty) => {
            items.iter().all(|item| {
                <$message>::decode(item.as_slice())
                    .is_ok_and(|value| !value.id.is_empty() && value.resource_version > 0)
            })
        };
    }
    match method {
        "ListRepositoryConnections" => all_decode!(v1::RepositoryConnection),
        "ListRepositorySnapshots" => all_decode!(v1::RepositorySnapshot),
        "ListRuntimeProfiles" => all_decode!(v1::RuntimeProfile),
        "ListRuntimeProfileRevisions" => all_decode!(v1::RuntimeProfileRevision),
        "ListRuntimeArtifacts" => all_decode!(v1::RuntimeArtifact),
        "ListAutomationInstallations" => all_decode!(v1::AutomationInstallation),
        "ListDevelopmentTasks" => all_decode!(v1::DevelopmentTask),
        "ListDevelopmentAttempts" => all_decode!(v1::DevelopmentAttempt),
        "ListCandidates" => all_decode!(v1::Candidate),
        "ListChangeRequests" => all_decode!(v1::ChangeRequest),
        "ListAssessments" => all_decode!(v1::Assessment),
        "ListEvidenceReferences" => all_decode!(v1::EvidenceReference),
        _ => false,
    }
}

fn encode_page_token(
    snapshot: &str,
    secret: &str,
    offset: usize,
) -> Result<String, CleanStoreError> {
    let token = PageToken {
        snapshot: snapshot.to_owned(),
        offset,
        proof: page_token_proof(secret, snapshot, offset),
    };
    let json = serde_json::to_vec(&token).map_err(|_| CleanStoreError::CorruptState)?;
    Ok(URL_SAFE_NO_PAD.encode(json))
}

fn decode_page_token(token: &str) -> Result<PageToken, CleanStoreError> {
    let json = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| CleanStoreError::InvalidPageToken)?;
    serde_json::from_slice(&json).map_err(|_| CleanStoreError::InvalidPageToken)
}

fn page_token_proof(secret: &str, snapshot: &str, offset: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"ayni.orchestrator.page.v1\0");
    hasher.update(secret.as_bytes());
    hasher.update([0]);
    hasher.update(snapshot.as_bytes());
    hasher.update([0]);
    hasher.update(offset.to_string().as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn encode_filter(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn page_size(page: Option<&v1::PageRequest>) -> usize {
    page.map_or(DEFAULT_PAGE_SIZE, |value| {
        if value.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            usize::try_from(value.page_size)
                .unwrap_or(MAX_PAGE_SIZE)
                .min(MAX_PAGE_SIZE)
        }
    })
}

fn append_event<M: Message>(
    transaction: &Transaction<'_>,
    service: &str,
    resource_type: &str,
    resource_id: &str,
    resource_version: u64,
    repository_connection_id: Option<&str>,
    parent_id: Option<&str>,
    value: &M,
) -> Result<u64, CleanStoreError> {
    transaction
        .execute(
            "INSERT INTO resource_events(
                service,resource_type,resource_id,resource_version,
                repository_connection_id,parent_id,payload,created_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                service,
                resource_type,
                resource_id,
                i64::try_from(resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                repository_connection_id,
                parent_id,
                value.encode_to_vec(),
                unix_seconds()?
            ],
        )
        .map_err(CleanStoreError::Database)?;
    let cursor = transaction.last_insert_rowid();
    prune_events(transaction, service)?;
    u64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)
}

fn prune_events(transaction: &Transaction<'_>, service: &str) -> Result<(), CleanStoreError> {
    let offset =
        i64::try_from(MAX_EVENTS_PER_SERVICE).map_err(|_| CleanStoreError::CorruptState)?;
    let cutoff: Option<i64> = transaction
        .query_row(
            "SELECT cursor FROM resource_events WHERE service=?1
             ORDER BY cursor DESC LIMIT 1 OFFSET ?2",
            params![service, offset],
            |row| row.get(0),
        )
        .optional()
        .map_err(CleanStoreError::Database)?;
    if let Some(cutoff) = cutoff {
        transaction
            .execute(
                "DELETE FROM resource_events WHERE service=?1 AND cursor<=?2",
                params![service, cutoff],
            )
            .map_err(CleanStoreError::Database)?;
    }
    let first: i64 = transaction
        .query_row(
            "SELECT COALESCE(MIN(cursor),0) FROM resource_events WHERE service=?1",
            [service],
            |row| row.get(0),
        )
        .map_err(CleanStoreError::Database)?;
    transaction
        .execute(
            "INSERT INTO event_watermarks(service,first_cursor) VALUES (?1,?2)
             ON CONFLICT(service) DO UPDATE SET first_cursor=excluded.first_cursor",
            params![service, first],
        )
        .map_err(CleanStoreError::Database)?;
    Ok(())
}
