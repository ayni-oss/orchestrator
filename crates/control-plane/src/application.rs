//! Transport-neutral product operations shared by HTTP and gRPC adapters.

use crate::{
    AppState,
    development::{DevelopmentInput, DevelopmentPhase, DevelopmentTask, RepositoryBinding},
    model_catalog::AvailableModels,
    model_usage::ModelUsageSummary,
    orchestration::{Store, StoreError},
    preparation::{PreparationView, PrepareRepositoryInput, RepositoryView},
    publication::{EvidenceView, EvidenceViewError},
};
use ayni_orchestrator_domain::{IdempotencyKey, TaskId};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: usize = 100;
const MAX_PAGE_TOKEN_BYTES: usize = 512;
const PAGE_TOKEN_LIFETIME_SECONDS: i64 = 15 * 60;

/// One validated product-list page request.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PageRequest {
    pub page_size: u32,
    pub page_token: String,
}

/// One bounded product-list page.
#[derive(Debug)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_page_token: String,
}

/// Bounded development-task projection used by list transports and snapshots.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DevelopmentTaskListItem {
    pub id: TaskId,
    pub repository_id: String,
    pub repository_owner: String,
    pub repository_name: String,
    pub start_revision: String,
    pub request: String,
    pub model_provider: String,
    pub model: String,
    pub retry_of: Option<TaskId>,
    pub created_at: i64,
    pub updated_at: i64,
    pub phase: DevelopmentPhase,
    pub candidate_commit: Option<String>,
    pub draft_pr_url: Option<String>,
    pub failure: Option<String>,
    pub failure_reason_code: Option<String>,
    pub model_usage: ModelUsageSummary,
    pub resource_version: u64,
}

impl From<DevelopmentTask> for DevelopmentTaskListItem {
    fn from(task: DevelopmentTask) -> Self {
        let failure_reason_code = task
            .lifecycle
            .iter()
            .rev()
            .find_map(|event| event.reason_code.clone());
        Self {
            id: task.id,
            repository_id: task.repository.id,
            repository_owner: task.repository.owner,
            repository_name: task.repository.repository,
            start_revision: task.input.start_revision,
            request: task.input.request,
            model_provider: task.agent_profile.provider,
            model: task.agent_profile.model,
            retry_of: task.retry_of,
            created_at: task.created_at,
            updated_at: task.updated_at,
            phase: task.phase,
            candidate_commit: task.candidate_commit,
            draft_pr_url: task.draft_pr_url,
            failure: task.failure,
            failure_reason_code,
            model_usage: task.model_usage.total,
            resource_version: task.resource_version,
        }
    }
}

struct PageSnapshot {
    item_ids: Vec<String>,
    item_values: Option<String>,
    offset: usize,
}

#[derive(Deserialize, Serialize)]
struct ProviderSnapshot {
    provider: String,
    observed_at: u64,
    models: Vec<crate::AvailableModel>,
}

/// Stable application error categories independent of either transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Transport cancellation/deadline paths construct these outside application calls.
pub enum ErrorReason {
    InvalidIdentifier,
    InvalidInput,
    IdempotencyKeyReused,
    ResourceNotFound,
    Conflict,
    FailedPrecondition,
    TaskNotActionable,
    ProviderUnavailable,
    ModelUnavailable,
    RepositoryAccess,
    EvidenceUnavailable,
    DependencyUnavailable,
    Cancelled,
    DeadlineExceeded,
    Internal,
    InvalidSourceRevision,
    InvalidLockDigest,
    RepositoryNotReady,
    TaskPhaseNotEligible,
    TaskDeadlineExpired,
}

/// A bounded, safe failure returned by application operations.
#[derive(Debug)]
pub struct ApplicationError {
    pub reason: ErrorReason,
    pub message: String,
    pub resource_type: Option<&'static str>,
    pub resource_id: Option<String>,
}

impl std::fmt::Display for ApplicationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ApplicationError {}

impl ApplicationError {
    fn new(reason: ErrorReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: truncate_utf8(message.into(), 512),
            resource_type: None,
            resource_id: None,
        }
    }

    fn resource(
        reason: ErrorReason,
        message: impl Into<String>,
        resource_type: &'static str,
        resource_id: impl Into<String>,
    ) -> Self {
        Self {
            reason,
            message: truncate_utf8(message.into(), 512),
            resource_type: Some(resource_type),
            resource_id: Some(truncate_utf8(resource_id.into(), 512)),
        }
    }

    pub(crate) fn from_store(error: StoreError) -> Self {
        match error {
            StoreError::RepositoryNotFound(id) => Self::resource(
                ErrorReason::ResourceNotFound,
                "repository was not found",
                "repository",
                id,
            ),
            StoreError::PreparationNotFound(id) => Self::resource(
                ErrorReason::ResourceNotFound,
                "preparation was not found",
                "preparation",
                id,
            ),
            StoreError::RepositoryNotReady => Self::new(
                ErrorReason::RepositoryNotReady,
                "repository revision is not ready; prepare it before submitting a task",
            ),
            StoreError::TaskPhaseNotEligible => Self::new(
                ErrorReason::TaskPhaseNotEligible,
                "development task phase is not eligible for this action",
            ),
            StoreError::TaskDeadlineExpired => Self::new(
                ErrorReason::TaskDeadlineExpired,
                "development task deadline has expired",
            ),
            StoreError::TaskNotFound(id) => Self::resource(
                ErrorReason::ResourceNotFound,
                "development task was not found",
                "development_task",
                id.to_string(),
            ),
            StoreError::AttemptNotFound(id) => Self::resource(
                ErrorReason::ResourceNotFound,
                "attempt was not found",
                "attempt",
                id.to_string(),
            ),
            StoreError::ActionKeyReused(_) => Self::new(
                ErrorReason::IdempotencyKeyReused,
                "idempotency key was reused with different request content",
            ),
            StoreError::RetryNotAllowed(_) | StoreError::MaxAttemptsReached { .. } => Self::new(
                ErrorReason::TaskPhaseNotEligible,
                "task phase is not eligible for this action",
            ),
            StoreError::TaskCancelled(id) => Self::resource(
                ErrorReason::TaskNotActionable,
                "cancelled task is not actionable",
                "development_task",
                id.to_string(),
            ),
            StoreError::InvalidIdentity(source) => {
                let message = source.to_string();
                let reason = if message.starts_with("Git revision") {
                    ErrorReason::InvalidSourceRevision
                } else if message.starts_with("SHA-256") {
                    ErrorReason::InvalidLockDigest
                } else {
                    ErrorReason::InvalidIdentifier
                };
                Self::new(reason, message)
            }
            StoreError::InvalidLifecycleData(_) => {
                Self::new(ErrorReason::Internal, "durable lifecycle state is invalid")
            }
            StoreError::InvalidChangeConstraints(source) => {
                Self::new(ErrorReason::InvalidInput, source.to_string())
            }
            StoreError::InvalidInput(message) => Self::new(ErrorReason::InvalidInput, message),
            StoreError::CorruptState(_) => {
                Self::new(ErrorReason::Internal, "durable state is corrupt")
            }
            StoreError::Transport(_) => Self::new(
                ErrorReason::RepositoryAccess,
                "repository execution dependency is unavailable",
            ),
            other => Self::new(ErrorReason::Internal, other.to_string()),
        }
    }
}

/// Canonical task-submission input accepted by every product transport.
pub struct CreateDevelopmentTaskCommand {
    pub idempotency_key: String,
    pub repository_id: String,
    pub model: String,
    pub start_revision: String,
    pub request: String,
    pub acceptance_criteria: Vec<String>,
    pub change_constraints: ayni_orchestrator_domain::ChangeConstraints,
    pub deadline_seconds: Option<u64>,
}

/// Product use cases. Adapters perform only protocol decoding/encoding.
#[derive(Clone)]
pub struct Application {
    state: AppState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LegacyRetryMethod {
    Retry,
    ProvideInput,
}

impl Application {
    pub(crate) const fn new(state: AppState) -> Self {
        Self { state }
    }

    pub(crate) fn list_repositories(
        &self,
        page: &PageRequest,
    ) -> Result<Page<RepositoryView>, ApplicationError> {
        let repositories = self.with_store(|store| store.repository_views())?;
        self.page_items(repositories, page, "repositories", "", |repository| {
            repository.binding.id.clone()
        })
    }

    pub(crate) fn get_repository(&self, id: &str) -> Result<RepositoryView, ApplicationError> {
        validate_repository_id(id)?;
        self.with_store(|store| store.repository_view(id))
            .map_err(|error| normalize_missing(error, "repository", id))
    }

    pub(crate) fn save_repository(
        &self,
        key: String,
        repository: &RepositoryBinding,
    ) -> Result<RepositoryView, ApplicationError> {
        let key = parse_key(key)?;
        self.with_store(|store| store.save_repository(&key, repository))
    }

    /// Preserves the migration HTTP endpoint's replace semantics. Native gRPC
    /// callers use `save_repository`, whose explicit key provides durable replay.
    pub(crate) fn replace_repository(
        &self,
        repository: &RepositoryBinding,
    ) -> Result<RepositoryView, ApplicationError> {
        self.with_store(|store| {
            store.put_repository_binding(repository)?;
            store.repository_view(&repository.id)
        })
    }

    pub(crate) fn list_preparations(
        &self,
        repository_id: &str,
        page: &PageRequest,
    ) -> Result<Page<PreparationView>, ApplicationError> {
        validate_repository_id(repository_id)?;
        let preparations = self
            .with_store(|store| store.preparations(repository_id))
            .map_err(|error| normalize_missing(error, "repository", repository_id))?;
        self.page_items(
            preparations,
            page,
            "preparations",
            repository_id,
            |preparation| preparation.id.clone(),
        )
    }

    pub(crate) fn get_preparation(&self, id: &str) -> Result<PreparationView, ApplicationError> {
        validate_preparation_id(id)?;
        self.with_store(|store| store.preparation(id))
            .map_err(|error| normalize_missing(error, "preparation", id))
    }

    pub(crate) fn prepare_repository(
        &self,
        key: String,
        repository_id: &str,
        revision: String,
    ) -> Result<PreparationView, ApplicationError> {
        validate_repository_id(repository_id)?;
        let raw_key = parse_key(key)?;
        let key = if self
            .with_store(|store| store.preparation_action(&raw_key))?
            .is_some()
        {
            raw_key
        } else {
            scoped_key("prepare_repository", &raw_key)?
        };
        self.with_store(|store| {
            store.prepare_repository(repository_id, &key, &PrepareRepositoryInput { revision })
        })
        .map_err(|error| normalize_missing(error, "repository", repository_id))
    }

    pub(crate) async fn list_provider_models(
        &self,
        page: &PageRequest,
    ) -> Result<(AvailableModels, Page<crate::AvailableModel>), ApplicationError> {
        let size = page_size(page)?;
        let (snapshot, offset) = if page.page_token.is_empty() {
            let catalog = self.state.model_catalog.as_ref().ok_or_else(|| {
                ApplicationError::new(
                    ErrorReason::ProviderUnavailable,
                    "provider model catalog is unavailable",
                )
            })?;
            let available = catalog.available().await.map_err(|_| {
                ApplicationError::new(
                    ErrorReason::ProviderUnavailable,
                    "provider model discovery failed",
                )
            })?;
            (
                ProviderSnapshot {
                    provider: available.provider.to_owned(),
                    observed_at: available.observed_at,
                    models: available.models,
                },
                0,
            )
        } else {
            let persisted = self
                .with_store(|store| {
                    store.list_snapshot(&page.page_token, "provider_models", "openai")
                })?
                .ok_or_else(|| {
                    ApplicationError::new(
                        ErrorReason::InvalidInput,
                        "page_token is invalid or expired",
                    )
                })?;
            let values = persisted.item_values.ok_or_else(|| {
                ApplicationError::new(
                    ErrorReason::InvalidInput,
                    "page_token does not contain a provider snapshot",
                )
            })?;
            let snapshot: ProviderSnapshot = serde_json::from_str(&values).map_err(|_| {
                ApplicationError::new(ErrorReason::Internal, "provider snapshot is corrupt")
            })?;
            let model_ids = snapshot
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>();
            if snapshot.provider != "openai"
                || model_ids
                    != persisted
                        .item_ids
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
            {
                return Err(ApplicationError::new(
                    ErrorReason::Internal,
                    "provider snapshot does not match its durable cursor",
                ));
            }
            (snapshot, persisted.offset)
        };
        if offset > snapshot.models.len() {
            return Err(ApplicationError::new(
                ErrorReason::InvalidInput,
                "page_token cursor is outside the provider snapshot",
            ));
        }
        let end = offset.saturating_add(size).min(snapshot.models.len());
        let models = snapshot.models[offset..end].to_vec();
        let next_page_token = if end < snapshot.models.len() {
            let item_ids = snapshot
                .models
                .iter()
                .map(|model| model.id.clone())
                .collect::<Vec<_>>();
            let values = serde_json::to_string(&snapshot).map_err(|_| {
                ApplicationError::new(ErrorReason::Internal, "could not persist provider snapshot")
            })?;
            self.with_store(|store| {
                store.save_list_snapshot("provider_models", "openai", &item_ids, Some(&values), end)
            })?
        } else {
            String::new()
        };
        let available = AvailableModels {
            provider: "openai",
            models: snapshot.models,
            observed_at: snapshot.observed_at,
        };
        Ok((
            available,
            Page {
                items: models,
                next_page_token,
            },
        ))
    }

    pub(crate) fn list_development_tasks(
        &self,
        phase: Option<DevelopmentPhase>,
        page: &PageRequest,
    ) -> Result<Page<DevelopmentTaskListItem>, ApplicationError> {
        let filter = phase.map_or_else(String::new, |value| value.as_str().to_owned());
        let mut tasks = self.with_store(|store| store.development_tasks_all())?;
        if page.page_token.is_empty()
            && let Some(phase) = phase
        {
            tasks.retain(|task| task.phase == phase);
        }
        let tasks = tasks
            .into_iter()
            .map(DevelopmentTaskListItem::from)
            .collect();
        self.page_items(tasks, page, "development_tasks", &filter, |task| {
            task.id.to_string()
        })
    }

    pub(crate) fn migration_development_tasks(
        &self,
    ) -> Result<Vec<DevelopmentTask>, ApplicationError> {
        self.with_store(|store| store.development_tasks_all())
    }

    pub(crate) fn get_development_task(
        &self,
        task_id: &str,
    ) -> Result<DevelopmentTask, ApplicationError> {
        let id = parse_task_id(task_id)?;
        self.with_store(|store| store.development_task(id))
    }

    pub(crate) async fn create_development_task(
        &self,
        command: CreateDevelopmentTaskCommand,
    ) -> Result<DevelopmentTask, ApplicationError> {
        let raw_key = parse_key(command.idempotency_key)?;
        let profile_id = Store::discovered_model_profile_id(&command.model);
        if command.model.is_empty()
            || command.model.len() > 128
            || !command
                .model
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b'"' && byte != b'\\')
        {
            return Err(ApplicationError::new(
                ErrorReason::InvalidInput,
                "model must be 1-128 safe ASCII bytes",
            ));
        }
        let input = DevelopmentInput {
            repository_id: command.repository_id,
            profile_id,
            start_revision: command.start_revision,
            request: command.request,
            acceptance_criteria: command.acceptance_criteria,
            change_constraints: command.change_constraints,
            deadline_seconds: command.deadline_seconds,
        };
        input.validate().map_err(ApplicationError::from_store)?;
        let legacy = self.with_store(|store| store.development_action(&raw_key))?;
        if legacy.as_ref().is_some_and(|(kind, _)| kind == "submit")
            && let Some(task) =
                self.with_store(|store| store.existing_development_submission(&raw_key, &input))?
        {
            return Ok(task);
        }
        let key = scoped_key("create_development_task", &raw_key)?;
        if let Some(task) =
            self.with_store(|store| store.existing_development_submission(&key, &input))?
        {
            return Ok(task);
        }
        let catalog = self.state.model_catalog.as_ref().ok_or_else(|| {
            ApplicationError::new(
                ErrorReason::ProviderUnavailable,
                "provider model catalog is unavailable",
            )
        })?;
        if !catalog.contains(&command.model).await.map_err(|_| {
            ApplicationError::new(
                ErrorReason::ProviderUnavailable,
                "provider model discovery failed",
            )
        })? {
            return Err(ApplicationError::new(
                ErrorReason::ModelUnavailable,
                "selected model is not available from the provider",
            ));
        }
        self.with_store(|store| store.put_discovered_model(&command.model))?;
        self.with_store(|store| store.submit_development(&key, &input))
    }

    pub(crate) fn cancel_development_task(
        &self,
        key: String,
        task_id: &str,
    ) -> Result<DevelopmentTask, ApplicationError> {
        let id = parse_task_id(task_id)?;
        let raw_key = parse_key(key)?;
        let legacy = self.with_store(|store| store.development_action(&raw_key))?;
        let key = if legacy.as_ref().is_some_and(|(kind, _)| kind == "cancel") {
            // Let the store compare the target so a same-method key reused for
            // another task remains a conflict instead of becoming a new key.
            raw_key
        } else {
            scoped_key("cancel_development_task", &raw_key)?
        };
        self.with_store(|store| store.cancel_development(id, &key))
    }

    pub(crate) fn retry_development_task(
        &self,
        key: String,
        task_id: &str,
    ) -> Result<DevelopmentTask, ApplicationError> {
        let id = parse_task_id(task_id)?;
        let raw_key = parse_key(key)?;
        let legacy = self.with_store(|store| store.development_action(&raw_key))?;
        let legacy_method = legacy
            .map(|(kind, target)| self.legacy_retry_method(&kind, target))
            .transpose()?
            .flatten();
        let key = if legacy_method == Some(LegacyRetryMethod::Retry) {
            // The store compares the original source and request shape. Keeping
            // the raw key here preserves same-method conflict semantics.
            raw_key
        } else {
            scoped_key("retry_development_task", &raw_key)?
        };
        self.with_store(|store| store.retry_development(id, &key, None))
    }

    pub(crate) fn provide_development_task_input(
        &self,
        key: String,
        task_id: &str,
        input: &str,
    ) -> Result<DevelopmentTask, ApplicationError> {
        let id = parse_task_id(task_id)?;
        let raw_key = parse_key(key)?;
        let legacy = self.with_store(|store| store.development_action(&raw_key))?;
        let legacy_method = legacy
            .map(|(kind, target)| self.legacy_retry_method(&kind, target))
            .transpose()?
            .flatten();
        let key = if legacy_method == Some(LegacyRetryMethod::ProvideInput) {
            // The store compares the original source and provided input. Keep
            // same-method legacy keys raw so changed content remains a conflict.
            raw_key
        } else {
            scoped_key("provide_development_task_input", &raw_key)?
        };
        self.with_store(|store| store.retry_development(id, &key, Some(input)))
    }

    fn legacy_retry_method(
        &self,
        kind: &str,
        target_id: TaskId,
    ) -> Result<Option<LegacyRetryMethod>, ApplicationError> {
        let Some(source_id) = kind.strip_prefix("development-retry:") else {
            return Ok(None);
        };
        let source_id = TaskId::parse(source_id).map_err(|_| {
            ApplicationError::new(ErrorReason::Internal, "durable lifecycle state is invalid")
        })?;
        let source = self
            .with_store(|store| store.development_task(source_id))
            .map_err(|_| {
                ApplicationError::new(ErrorReason::Internal, "durable lifecycle state is invalid")
            })?;
        let target = self
            .with_store(|store| store.development_task(target_id))
            .map_err(|_| {
                ApplicationError::new(ErrorReason::Internal, "durable lifecycle state is invalid")
            })?;
        if target.provided_input.is_some() || target.input.request != source.input.request {
            Ok(Some(LegacyRetryMethod::ProvideInput))
        } else {
            Ok(Some(LegacyRetryMethod::Retry))
        }
    }

    pub(crate) fn get_development_task_evidence(
        &self,
        task_id: &str,
    ) -> Result<EvidenceView, ApplicationError> {
        let task = self.get_development_task(task_id)?;
        let publisher = self.state.publisher().ok_or_else(|| {
            ApplicationError::new(
                ErrorReason::DependencyUnavailable,
                "development task evidence service is unavailable",
            )
        })?;
        publisher.evidence_view(&task).map_err(|error| match error {
            EvidenceViewError::Unavailable => ApplicationError::resource(
                ErrorReason::EvidenceUnavailable,
                "development task evidence is unavailable",
                "development_task_evidence",
                task_id,
            ),
            EvidenceViewError::Corrupt => ApplicationError::resource(
                ErrorReason::Internal,
                "development task evidence is corrupt",
                "development_task_evidence",
                task_id,
            ),
        })
    }

    fn page_items<T>(
        &self,
        items: Vec<T>,
        request: &PageRequest,
        method: &str,
        filter: &str,
        identity: impl Fn(&T) -> String,
    ) -> Result<Page<T>, ApplicationError>
    where
        T: DeserializeOwned + Serialize,
    {
        let size = page_size(request)?;
        let (ordered, item_ids, item_values, offset) = if request.page_token.is_empty() {
            let item_ids = items.iter().map(&identity).collect::<Vec<_>>();
            let item_values = serde_json::to_string(&items).map_err(|_| {
                ApplicationError::new(ErrorReason::Internal, "could not persist list snapshot")
            })?;
            (items, item_ids, item_values, 0)
        } else {
            let snapshot = self
                .with_store(|store| store.list_snapshot(&request.page_token, method, filter))?
                .ok_or_else(|| {
                    ApplicationError::new(
                        ErrorReason::InvalidInput,
                        "page_token is invalid or expired",
                    )
                })?;
            let item_values = snapshot.item_values.ok_or_else(|| {
                ApplicationError::new(
                    ErrorReason::InvalidInput,
                    "page_token does not contain a resource snapshot",
                )
            })?;
            let ordered: Vec<T> = serde_json::from_str(&item_values).map_err(|_| {
                ApplicationError::new(ErrorReason::Internal, "list snapshot is corrupt")
            })?;
            let restored_ids = ordered.iter().map(&identity).collect::<Vec<_>>();
            if restored_ids != snapshot.item_ids {
                return Err(ApplicationError::new(
                    ErrorReason::Internal,
                    "list snapshot does not match its durable cursor",
                ));
            }
            (ordered, snapshot.item_ids, item_values, snapshot.offset)
        };
        if offset > ordered.len() {
            return Err(ApplicationError::new(
                ErrorReason::InvalidInput,
                "page_token cursor is outside the list snapshot",
            ));
        }
        let end = offset.saturating_add(size).min(ordered.len());
        let has_next = end < ordered.len();
        let page = ordered
            .into_iter()
            .skip(offset)
            .take(end.saturating_sub(offset))
            .collect();
        let next_page_token = if has_next {
            self.with_store(|store| {
                store.save_list_snapshot(method, filter, &item_ids, Some(&item_values), end)
            })?
        } else {
            String::new()
        };
        Ok(Page {
            items: page,
            next_page_token,
        })
    }

    fn with_store<T>(
        &self,
        operation: impl FnOnce(&mut Store) -> Result<T, StoreError>,
    ) -> Result<T, ApplicationError> {
        let mut store = self.state.store.lock().map_err(|_| {
            ApplicationError::new(ErrorReason::Internal, "durable store lock was poisoned")
        })?;
        operation(&mut store).map_err(ApplicationError::from_store)
    }
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value.truncate(end);
    value
}

fn validate_repository_id(value: &str) -> Result<(), ApplicationError> {
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return Err(ApplicationError::new(
            ErrorReason::InvalidIdentifier,
            "repository_id must be a 1-64 byte lowercase slug",
        ));
    }
    Ok(())
}

fn validate_preparation_id(value: &str) -> Result<(), ApplicationError> {
    if value.len() != 69
        || !value.starts_with("prep-")
        || !value[5..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ApplicationError::new(
            ErrorReason::InvalidIdentifier,
            "preparation_id must be a canonical preparation identifier",
        ));
    }
    Ok(())
}

fn parse_key(value: String) -> Result<IdempotencyKey, ApplicationError> {
    IdempotencyKey::try_from(value).map_err(|_| {
        ApplicationError::new(
            ErrorReason::InvalidInput,
            "idempotency_key must be 1-128 safe ASCII bytes",
        )
    })
}

fn scoped_key(method: &str, key: &IdempotencyKey) -> Result<IdempotencyKey, ApplicationError> {
    // The supported unauthenticated profile has one implicit trusted-local
    // caller. Keep that scope explicit so an authenticated profile can replace
    // it with a principal identity without changing method isolation.
    let digest = Sha256::digest(format!("trusted-local\0{method}\0{}", key.as_str()).as_bytes());
    IdempotencyKey::try_from(format!("rpc-{digest:x}"))
        .map_err(|error| ApplicationError::new(ErrorReason::Internal, error.to_string()))
}

fn parse_task_id(value: &str) -> Result<TaskId, ApplicationError> {
    TaskId::parse(value)
        .map_err(|error| ApplicationError::new(ErrorReason::InvalidIdentifier, error.to_string()))
}

fn normalize_missing(
    error: ApplicationError,
    resource_type: &'static str,
    resource_id: &str,
) -> ApplicationError {
    if error.reason == ErrorReason::ResourceNotFound {
        ApplicationError::resource(
            ErrorReason::ResourceNotFound,
            format!("{resource_type} was not found"),
            resource_type,
            resource_id,
        )
    } else {
        error
    }
}

fn page_size(request: &PageRequest) -> Result<usize, ApplicationError> {
    if request.page_size > u32::try_from(MAX_PAGE_SIZE).unwrap_or(u32::MAX) {
        return Err(ApplicationError::new(
            ErrorReason::InvalidInput,
            "page_size must be between 1 and 100, or zero for the default",
        ));
    }
    if request.page_token.len() > MAX_PAGE_TOKEN_BYTES {
        return Err(ApplicationError::new(
            ErrorReason::InvalidInput,
            "page_token exceeds 512 bytes",
        ));
    }
    Ok(if request.page_size == 0 {
        DEFAULT_PAGE_SIZE
    } else {
        usize::try_from(request.page_size).unwrap_or(MAX_PAGE_SIZE)
    })
}

impl Store {
    fn save_list_snapshot(
        &self,
        method: &str,
        filter: &str,
        item_ids: &[String],
        item_values: Option<&str>,
        offset: usize,
    ) -> Result<String, StoreError> {
        let now = crate::orchestration::unix_seconds()?;
        self.connection
            .execute(
                "DELETE FROM application_list_snapshots WHERE created_at < ?1",
                [now.saturating_sub(PAGE_TOKEN_LIFETIME_SECONDS)],
            )
            .map_err(StoreError::Database)?;
        let token = Uuid::now_v7().to_string();
        let item_ids = serde_json::to_string(item_ids)
            .map_err(|_| StoreError::InvalidInput("could not serialize list snapshot"))?;
        let offset = i64::try_from(offset)
            .map_err(|_| StoreError::InvalidInput("list snapshot offset is too large"))?;
        self.connection
            .execute(
                "INSERT INTO application_list_snapshots
                 (token, method, filter, item_ids, item_values, item_offset, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![token, method, filter, item_ids, item_values, offset, now],
            )
            .map_err(StoreError::Database)?;
        Ok(token)
    }

    fn list_snapshot(
        &self,
        token: &str,
        method: &str,
        filter: &str,
    ) -> Result<Option<PageSnapshot>, StoreError> {
        let parsed = Uuid::parse_str(token).ok();
        if token.len() != 36 || parsed.is_none_or(|value| value.to_string() != token) {
            return Ok(None);
        }
        let now = crate::orchestration::unix_seconds()?;
        let snapshot = self
            .connection
            .query_row(
                "SELECT item_ids, item_values, item_offset, created_at
                 FROM application_list_snapshots
                 WHERE token=?1 AND method=?2 AND filter=?3",
                params![token, method, filter],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(StoreError::Database)?;
        let Some((item_ids, item_values, offset, created_at)) = snapshot else {
            return Ok(None);
        };
        if created_at > now || now.saturating_sub(created_at) > PAGE_TOKEN_LIFETIME_SECONDS {
            return Ok(None);
        }
        let item_ids = serde_json::from_str(&item_ids)
            .map_err(|_| StoreError::CorruptState("invalid persisted list snapshot"))?;
        let offset = usize::try_from(offset)
            .map_err(|_| StoreError::CorruptState("invalid persisted list snapshot offset"))?;
        Ok(Some(PageSnapshot {
            item_ids,
            item_values,
            offset,
        }))
    }
}

#[cfg(test)]
#[path = "application_tests.rs"]
mod tests;
