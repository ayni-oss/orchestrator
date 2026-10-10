//! Durable clean-v1 resource storage.
#![allow(
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
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use uuid::Uuid;

pub(crate) const SCHEMA_VERSION: i64 = 10;
const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: usize = 100;
const PAGE_LIFETIME_SECONDS: i64 = 15 * 60;
const MAX_PAGE_SNAPSHOTS: usize = 128;
const MAX_PAGE_SNAPSHOT_ITEMS: usize = 10_000;
const MAX_PAGE_SNAPSHOT_BYTES: usize = 8 * 1_024 * 1_024;
const MAX_PAGE_RESPONSE_BYTES: usize = 3 * 1_024 * 1_024;
const MAX_EVENTS_PER_SERVICE: usize = 4_096;
const MAX_RECONCILE_BATCH: usize = 128;
const MAX_PENDING_ACTION_BATCH: usize = 256;
const CLEAN_TABLES: &[&str] = &[
    "certificate_operations",
    "certificates",
    "clean_schema",
    "connection_operations",
    "connections",
    "development_attempts",
    "development_tasks",
    "durable_actions",
    "event_watermarks",
    "page_snapshots",
    "project_repositories",
    "projects",
    "reconciliation_cursors",
    "repository_connections",
    "repository_snapshots",
    "resource_events",
    "runtime_artifacts",
    "runtime_profile_revisions",
    "runtime_profiles",
];

pub(crate) const PROJECT_SERVICE: &str = "projects";

pub(crate) const REPOSITORY_SERVICE: &str = "repositories";
pub(crate) const PROFILE_SERVICE: &str = "runtime_profiles";
pub(crate) const DEVELOPMENT_SERVICE: &str = "development_tasks";

#[derive(Debug, Error)]
pub enum CleanStoreError {
    #[error("could not create the durable-store directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("durable SQLite operation failed")]
    Database(#[source] rusqlite::Error),
    #[error("could not start the database worker")]
    DatabaseWorker(#[source] std::io::Error),
    #[error("the database worker is unavailable")]
    StoreUnavailable,
    #[error(
        "the database contains the retired pilot schema; back it up if needed, then run the documented guarded purge/reset procedure"
    )]
    RetiredPilotSchema,
    #[error("unsupported clean-v1 schema version {0}")]
    UnsupportedSchemaVersion(i64),
    #[error("runtime infrastructure configuration is invalid: {0}")]
    RuntimeConfiguration(&'static str),
    #[error("durable clean-v1 state is corrupt")]
    CorruptState,
    #[error("resource {resource_type} {resource_id} was not found")]
    NotFound {
        resource_type: &'static str,
        resource_id: String,
    },
    #[error("idempotency key was reused with different request content")]
    IdempotencyConflict,
    #[error("runtime profile {profile_id} already owns this managed repository lock")]
    ManagedLockProfileAlreadyExists { profile_id: String },
    #[error("the repository already belongs to an active project")]
    RepositoryAlreadyAssigned,
    #[error("the project is archived")]
    ProjectArchived,
    #[error("{0}")]
    RepositoryRemovalBlocked(&'static str),
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
    pub idempotency_key: String,
    pub state: DurableActionState,
    pub request_payload: Vec<u8>,
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
    coding_turns: u32,
    quality_repair_limit: u32,
}

mod connection_dependencies;
mod profile_identity;
mod projects;
mod repository_connections;
mod repository_removal;
mod runtime_certificate;
mod runtime_profiles;
mod runtime_registry;
mod store;
mod summaries;
pub(crate) mod task_admission;
mod task_lifecycle;
mod validation;
use validation::{
    collect_bounded_payloads, get_payload, persist_attempt_update, persist_task_update,
    query_payloads, valid_version_and_cursor, validate_durable_actions, validate_indexed_rows,
    validate_page_snapshot_rows, validate_resource_events, validate_resource_tables,
};
mod transitions_schema;
use transitions_schema::{
    action_row, decode, initialize_schema, is_terminal_attempt, map_constraint, resource_id,
    task_state_for_attempt, timestamp, unix_seconds, unix_timestamp, valid_attempt_transition,
    valid_profile_transition, valid_repository_transition,
};
mod payloads;
use payloads::{
    bounded_failure, equivalent_failure, safe_metadata, same_artifact_report,
    valid_artifact_report, valid_git_revision, valid_model_usage,
};
mod pagination_events;
use pagination_events::{
    append_event, decode_page_token, encode_filter, encode_page_token, page_items_decode,
    page_size, page_token_proof, provider_key,
};
mod coding_reconciliation;
mod runtime_reconciliation;

#[cfg(test)]
mod tests {
    use super::transitions_schema::{existing_tables, valid_artifact_transition};
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
            use_ayni: None,
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
            registry_connection_id: String::new(),
            image_namespace: String::new(),
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
                            runtime_source_digest: None,
                            lock_fingerprint: "c".repeat(64),
                            ayni_version: "0.15.0".to_owned(),
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
            verified_metadata: vec![
                v1::VerifiedMetadata {
                    key: "ayni-contract".into(),
                    value: "0.15.0".into(),
                },
                v1::VerifiedMetadata {
                    key: "builder-image".into(),
                    value: format!("ghcr.io/ayni-oss/ayni-builder@{DIGEST}"),
                },
            ],
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
        store.create_project(
            "project-scope",
            &v1::CreateProjectRequest {
                idempotency_key: "project".into(),
                display_name: "Sandbox".into(),
                repository_connection_id: repository.id.clone(),
                repository_connection_ids: vec![],
            },
        )?;
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
        store: &CleanStore,
        repository_id: &str,
        profile_id: &str,
        key: &str,
    ) -> Result<v1::CreateDevelopmentTaskRequest, CleanStoreError> {
        let project_id: String = store.connection.query_row("SELECT project_id FROM project_repositories WHERE repository_connection_id=?1 AND active=1", [repository_id], |r| r.get(0)).map_err(CleanStoreError::Database)?;
        Ok(v1::CreateDevelopmentTaskRequest {
            idempotency_key: key.to_owned(),
            request: Some(v1::DevelopmentTaskRequest {
                coding_turns: None,
                project_id,
                repository_connection_id: repository_id.to_owned(),
                repository_revision: REVISION.to_owned(),
                runtime_profile_id: profile_id.to_owned(),
                agent: Some(v1::AgentSelection {
                    provider: "openai".to_owned(),
                    agent: "coding-agent".to_owned(),
                    model: "gpt-5.6-sol".to_owned(),
                }),
                request: "Make the bounded change".to_owned(),
                acceptance_criteria: vec!["Tests pass".to_owned()],
                change_constraints: Some(v1::ChangeConstraints::default()),
                deadline: Some(Duration {
                    seconds: 3_600,
                    nanos: 0,
                }),
            }),
        })
    }

    mod lifecycle;
    mod projects;
    mod recovery;
    mod runtime;
    mod runtime_certificate;
    mod runtime_registry;
    mod summaries;
}

mod project_validation;
use project_validation::{valid_project, validate_projects};

#[cfg(test)]
use transitions_schema::add_duration;
pub(crate) use transitions_schema::request_digest;

pub(crate) mod database;
pub(crate) mod task_cancellation;

pub(crate) mod runtime_activation;

pub(crate) mod runtime_selection;

pub(crate) mod task_retry;

mod certificates;
mod connections;

mod builder_cutover;

mod preparation_migration;

mod summary_index;
