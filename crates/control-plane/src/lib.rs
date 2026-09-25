//! HTTP API and durable orchestration surface for Ayni Orchestrator.

mod api;
mod development;
mod development_jobs;
mod development_lifecycle;
mod github;
mod orchestration;
mod preparation;
mod preparation_jobs;
mod publication;
mod ui;

use api::{ApiError, HealthResponse};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use ayni_orchestrator_domain::{AttemptId, AttemptOutcome, IdempotencyKey, TaskId};
use development::{AgentProfile, DevelopmentInput, DevelopmentTask, RepositoryBinding};
use orchestration::{Store, StoreError, StoreHandle, TaskInput, TaskView};
use preparation::{PreparationView, PrepareRepositoryInput, RepositoryView};
use serde::Deserialize;
use std::{
    env,
    net::SocketAddr,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, Mutex},
};
use thiserror::Error;

pub use development_jobs::DevelopmentReconciler;
pub use orchestration::{JobTransport, KubernetesJobTransport, Reconciler, TransportError};
pub use preparation_jobs::PreparationReconciler;
pub use publication::PublisherState;

/// Configuration validated before the control plane starts accepting traffic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Configuration {
    bind_address: SocketAddr,
    database_path: PathBuf,
    kubernetes_namespace: String,
    worker_image: String,
    max_attempts: u32,
}

impl Configuration {
    /// The default address used when no address is configured.
    pub const DEFAULT_BIND_ADDRESS: &'static str = "0.0.0.0:8080";
    /// Persistent task database path inside the control-plane pod.
    pub const DEFAULT_DATABASE_PATH: &'static str = "/data/orchestrator.sqlite";
    /// Default namespace for attempt-owned Jobs.
    pub const DEFAULT_KUBERNETES_NAMESPACE: &'static str = "ayni-orchestrator-dev";
    /// The worker image reference loaded into the selected Kubernetes runtime.
    pub const DEFAULT_WORKER_IMAGE: &'static str = "local/ayni-orchestrator-worker:replace-me";
    /// Default inclusive cap for attempts created for one task.
    pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;

    /// Reads configuration from the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigurationError`] for invalid socket or namespace values.
    pub fn from_environment() -> Result<Self, ConfigurationError> {
        let bind_address = env::var("ORCHESTRATOR_BIND_ADDRESS")
            .unwrap_or_else(|_| Self::DEFAULT_BIND_ADDRESS.to_owned());
        let database_path = env::var("ORCHESTRATOR_DATABASE_PATH")
            .unwrap_or_else(|_| Self::DEFAULT_DATABASE_PATH.to_owned());
        let kubernetes_namespace = env::var("ORCHESTRATOR_KUBERNETES_NAMESPACE")
            .unwrap_or_else(|_| Self::DEFAULT_KUBERNETES_NAMESPACE.to_owned());
        let worker_image = env::var("ORCHESTRATOR_WORKER_IMAGE")
            .unwrap_or_else(|_| Self::DEFAULT_WORKER_IMAGE.to_owned());
        let max_attempts = env::var("ORCHESTRATOR_MAX_ATTEMPTS")
            .ok()
            .map(|value| value.parse())
            .transpose()
            .map_err(ConfigurationError::InvalidMaxAttempts)?
            .unwrap_or(Self::DEFAULT_MAX_ATTEMPTS);
        Self::from_values(
            &bind_address,
            PathBuf::from(database_path),
            kubernetes_namespace,
            worker_image,
            max_attempts,
        )
    }

    /// Validates explicit configuration values.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigurationError`] for invalid socket or namespace values.
    pub fn from_values(
        bind_address: &str,
        database_path: PathBuf,
        kubernetes_namespace: String,
        worker_image: String,
        max_attempts: u32,
    ) -> Result<Self, ConfigurationError> {
        let bind_address = SocketAddr::from_str(bind_address).map_err(|source| {
            ConfigurationError::InvalidBindAddress {
                value: bind_address.to_owned(),
                source,
            }
        })?;
        if database_path.as_os_str().is_empty() || !database_path.is_absolute() {
            return Err(ConfigurationError::InvalidDatabasePath(database_path));
        }
        if kubernetes_namespace.is_empty()
            || kubernetes_namespace.len() > 63
            || !kubernetes_namespace.bytes().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == b'-'
            })
        {
            return Err(ConfigurationError::InvalidNamespace(kubernetes_namespace));
        }
        if worker_image.is_empty()
            || worker_image.len() > 255
            || worker_image.contains(char::is_whitespace)
        {
            return Err(ConfigurationError::InvalidWorkerImage(worker_image));
        }
        if !(1..=10).contains(&max_attempts) {
            return Err(ConfigurationError::MaxAttemptsOutOfRange(max_attempts));
        }
        Ok(Self {
            bind_address,
            database_path,
            kubernetes_namespace,
            worker_image,
            max_attempts,
        })
    }

    /// Returns the address on which the control plane listens.
    #[must_use]
    pub const fn bind_address(&self) -> SocketAddr {
        self.bind_address
    }
    /// Returns the durable state path.
    #[must_use]
    pub const fn database_path(&self) -> &PathBuf {
        &self.database_path
    }
    /// Returns the namespace where attempt-owned Jobs are reconciled.
    #[must_use]
    pub fn kubernetes_namespace(&self) -> &str {
        &self.kubernetes_namespace
    }
    /// Returns the configured attempt worker image reference.
    #[must_use]
    pub fn worker_image(&self) -> &str {
        &self.worker_image
    }
    /// Returns the inclusive attempt limit applied to newly submitted tasks.
    #[must_use]
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }
}

/// A configuration value could not be validated.
#[derive(Debug, Error)]
pub enum ConfigurationError {
    /// `ORCHESTRATOR_BIND_ADDRESS` was not a valid address.
    #[error("ORCHESTRATOR_BIND_ADDRESS must be a socket address, got {value:?}")]
    InvalidBindAddress {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
    /// `ORCHESTRATOR_DATABASE_PATH` was not absolute.
    #[error("ORCHESTRATOR_DATABASE_PATH must be absolute, got {0}")]
    InvalidDatabasePath(PathBuf),
    /// `ORCHESTRATOR_KUBERNETES_NAMESPACE` was not a DNS-label-like namespace.
    #[error("ORCHESTRATOR_KUBERNETES_NAMESPACE must be a lowercase DNS label, got {0:?}")]
    InvalidNamespace(String),
    /// `ORCHESTRATOR_WORKER_IMAGE` was empty or contained whitespace.
    #[error("ORCHESTRATOR_WORKER_IMAGE must be a non-empty image reference, got {0:?}")]
    InvalidWorkerImage(String),
    /// `ORCHESTRATOR_MAX_ATTEMPTS` was not an unsigned integer.
    #[error("ORCHESTRATOR_MAX_ATTEMPTS must be an unsigned integer")]
    InvalidMaxAttempts(#[source] std::num::ParseIntError),
    /// The configured attempt cap is outside the bounded local profile.
    #[error("ORCHESTRATOR_MAX_ATTEMPTS must be between 1 and 10, got {0}")]
    MaxAttemptsOutOfRange(u32),
}

/// Shared API state, deliberately independent from the Kubernetes transport.
#[derive(Clone)]
pub struct AppState {
    store: StoreHandle,
    publisher: Option<Arc<PublisherState>>,
}

impl AppState {
    /// Opens the persistent store used by HTTP handlers and reconciliation.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the durable database cannot be opened.
    pub fn open(
        database_path: impl AsRef<std::path::Path>,
        max_attempts: u32,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            store: Arc::new(Mutex::new(Store::open(database_path, max_attempts)?)),
            publisher: None,
        })
    }

    #[must_use]
    pub fn with_publisher(mut self, publisher: Option<PublisherState>) -> Self {
        self.publisher = publisher.map(Arc::new);
        self
    }

    /// Returns the shared durable-store handle for the reconciler.
    #[must_use]
    pub fn store(&self) -> StoreHandle {
        Arc::clone(&self.store)
    }

    #[must_use]
    pub fn publisher(&self) -> Option<Arc<PublisherState>> {
        self.publisher.clone()
    }
}

/// Builds the M2 HTTP router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(ui::index))
        .route("/livez", get(liveness))
        .route("/readyz", get(readiness))
        .route("/v1/tasks", post(submit_task))
        .route("/v1/tasks/{task_id}", get(task))
        .route("/v1/tasks/{task_id}/retry", post(retry_task))
        .route("/v1/tasks/{task_id}/cancel", post(cancel_task))
        .route("/v1/attempts/{attempt_id}/reports", post(report_attempt))
        .route(
            "/v1/repository-bindings",
            get(repository_bindings).post(put_repository_binding),
        )
        .route(
            "/v1/repository-bindings/{repository_id}",
            get(repository_binding),
        )
        .route(
            "/v1/repository-bindings/{repository_id}/prepare",
            post(prepare_repository),
        )
        .route("/v1/preparations/{preparation_id}", get(preparation))
        .route(
            "/v1/agent-profiles",
            get(agent_profiles).post(put_agent_profile),
        )
        .route(
            "/v1/development-tasks",
            get(development_tasks).post(submit_development_task),
        )
        .route("/v1/development-tasks/{task_id}", get(development_task))
        .route(
            "/v1/development-tasks/{task_id}/cancel",
            post(cancel_development_task),
        )
        .route(
            "/v1/development-tasks/{task_id}/retry",
            post(retry_development_task),
        )
        .route(
            "/v1/development-tasks/{task_id}/provide-input",
            post(provide_development_input),
        )
        .route(
            "/v1/development-tasks/{task_id}/evidence",
            get(development_evidence),
        )
        .route(
            "/v1/development-tasks/{task_id}/publish",
            post(publication::publish),
        )
        .with_state(state)
}

async fn repository_bindings(
    State(state): State<AppState>,
) -> Result<Json<Vec<RepositoryView>>, ApiError> {
    Ok(Json(with_store(&state, |store| store.repository_views())?))
}

async fn repository_binding(
    State(state): State<AppState>,
    Path(repository_id): Path<String>,
) -> Result<Json<RepositoryView>, ApiError> {
    Ok(Json(with_store(&state, |store| {
        store.repository_view(&repository_id)
    })?))
}

async fn prepare_repository(
    State(state): State<AppState>,
    Path(repository_id): Path<String>,
    Json(request): Json<PrepareRepositoryRequest>,
) -> Result<(StatusCode, Json<PreparationView>), ApiError> {
    let key = parse_key(request.idempotency_key)?;
    let preparation = with_store(&state, |store| {
        store.prepare_repository(&repository_id, &key, &request.input)
    })?;
    Ok((StatusCode::ACCEPTED, Json(preparation)))
}

async fn preparation(
    State(state): State<AppState>,
    Path(preparation_id): Path<String>,
) -> Result<Json<PreparationView>, ApiError> {
    Ok(Json(with_store(&state, |store| {
        store.preparation(&preparation_id)
    })?))
}

async fn put_repository_binding(
    State(state): State<AppState>,
    Json(binding): Json<RepositoryBinding>,
) -> Result<StatusCode, ApiError> {
    with_store(&state, |store| store.put_repository_binding(&binding))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn agent_profiles(
    State(state): State<AppState>,
) -> Result<Json<Vec<AgentProfile>>, ApiError> {
    Ok(Json(with_store(&state, |store| store.agent_profiles())?))
}

async fn put_agent_profile(
    State(state): State<AppState>,
    Json(profile): Json<AgentProfile>,
) -> Result<StatusCode, ApiError> {
    with_store(&state, |store| store.put_agent_profile(&profile))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn development_tasks(
    State(state): State<AppState>,
) -> Result<Json<Vec<DevelopmentTask>>, ApiError> {
    Ok(Json(with_store(&state, |store| store.development_tasks())?))
}

async fn submit_development_task(
    State(state): State<AppState>,
    Json(request): Json<SubmitDevelopmentRequest>,
) -> Result<(StatusCode, Json<DevelopmentTask>), ApiError> {
    let key = parse_key(request.idempotency_key)?;
    let task = with_store(&state, |store| {
        store.submit_development(&key, &request.input)
    })?;
    Ok((StatusCode::ACCEPTED, Json(task)))
}

async fn development_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<DevelopmentTask>, ApiError> {
    let id = TaskId::parse(&task_id).map_err(ApiError::InvalidIdentifier)?;
    Ok(Json(with_store(&state, |store| {
        store.development_task(id)
    })?))
}

async fn cancel_development_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<ActionRequest>,
) -> Result<Json<DevelopmentTask>, ApiError> {
    let id = TaskId::parse(&task_id).map_err(ApiError::InvalidIdentifier)?;
    let key = parse_key(request.idempotency_key)?;
    Ok(Json(with_store(&state, |store| {
        store.cancel_development(id, &key)
    })?))
}

async fn retry_development_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<ActionRequest>,
) -> Result<(StatusCode, Json<DevelopmentTask>), ApiError> {
    let id = TaskId::parse(&task_id).map_err(ApiError::InvalidIdentifier)?;
    let key = parse_key(request.idempotency_key)?;
    let task = with_store(&state, |store| store.retry_development(id, &key, None))?;
    Ok((StatusCode::ACCEPTED, Json(task)))
}

async fn provide_development_input(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<ProvideInputRequest>,
) -> Result<(StatusCode, Json<DevelopmentTask>), ApiError> {
    let id = TaskId::parse(&task_id).map_err(ApiError::InvalidIdentifier)?;
    let key = parse_key(request.idempotency_key)?;
    let task = with_store(&state, |store| {
        store.retry_development(id, &key, Some(&request.input))
    })?;
    Ok((StatusCode::ACCEPTED, Json(task)))
}

async fn development_evidence(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<publication::EvidenceView>, ApiError> {
    let id = TaskId::parse(&task_id).map_err(ApiError::InvalidIdentifier)?;
    let task = with_store(&state, |store| store.development_task(id))?;
    let publisher = state.publisher().ok_or(ApiError::PublisherUnavailable)?;
    let evidence = publisher
        .evidence_view(&task)
        .map_err(ApiError::EvidenceUnavailable)?;
    Ok(Json(evidence))
}

async fn liveness() -> impl IntoResponse {
    (StatusCode::OK, Json(HealthResponse::live()))
}
async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse::ready(state.publisher.is_some())),
    )
}

async fn submit_task(
    State(state): State<AppState>,
    Json(request): Json<SubmitTaskRequest>,
) -> Result<(StatusCode, Json<TaskView>), ApiError> {
    let action_key = parse_key(request.idempotency_key)?;
    let task = with_store(&state, |store| store.submit(&action_key, &request.input))?;
    Ok((StatusCode::ACCEPTED, Json(task)))
}
async fn task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<TaskView>, ApiError> {
    let task_id = TaskId::parse(&task_id).map_err(ApiError::InvalidIdentifier)?;
    Ok(Json(with_store(&state, |store| store.task(task_id))?))
}
async fn retry_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<ActionRequest>,
) -> Result<Json<TaskView>, ApiError> {
    let task_id = TaskId::parse(&task_id).map_err(ApiError::InvalidIdentifier)?;
    let action_key = parse_key(request.idempotency_key)?;
    Ok(Json(with_store(&state, |store| {
        store.retry(task_id, &action_key)
    })?))
}
async fn cancel_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<ActionRequest>,
) -> Result<Json<TaskView>, ApiError> {
    let task_id = TaskId::parse(&task_id).map_err(ApiError::InvalidIdentifier)?;
    let action_key = parse_key(request.idempotency_key)?;
    Ok(Json(with_store(&state, |store| {
        store.cancel(task_id, &action_key)
    })?))
}
async fn report_attempt(
    State(state): State<AppState>,
    Path(attempt_id): Path<String>,
    Json(request): Json<ReportRequest>,
) -> Result<StatusCode, ApiError> {
    let attempt_id = AttemptId::parse(&attempt_id).map_err(ApiError::InvalidIdentifier)?;
    let report_key = parse_key(request.idempotency_key)?;
    with_store(&state, |store| {
        store.report(attempt_id, &report_key, request.outcome)
    })?;
    Ok(StatusCode::NO_CONTENT)
}

fn with_store<T>(
    state: &AppState,
    operation: impl FnOnce(&mut Store) -> Result<T, StoreError>,
) -> Result<T, ApiError> {
    let mut store = state
        .store
        .lock()
        .map_err(|_| ApiError::StoreLockPoisoned)?;
    operation(&mut store).map_err(ApiError::Store)
}
fn parse_key(value: String) -> Result<IdempotencyKey, ApiError> {
    IdempotencyKey::try_from(value).map_err(ApiError::InvalidIdentifier)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitTaskRequest {
    idempotency_key: String,
    #[serde(flatten)]
    input: TaskInput,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitDevelopmentRequest {
    idempotency_key: String,
    #[serde(flatten)]
    input: DevelopmentInput,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareRepositoryRequest {
    idempotency_key: String,
    #[serde(flatten)]
    input: PrepareRepositoryInput,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvideInputRequest {
    idempotency_key: String,
    input: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionRequest {
    idempotency_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportRequest {
    idempotency_key: String,
    outcome: AttemptOutcome,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    fn state() -> Result<AppState, StoreError> {
        Ok(AppState {
            store: Arc::new(Mutex::new(Store::in_memory()?)),
            publisher: None,
        })
    }
    #[test]
    fn accepts_durable_m2_configuration() -> Result<(), ConfigurationError> {
        let configuration = Configuration::from_values(
            "127.0.0.1:8080",
            PathBuf::from("/data/tasks.sqlite"),
            "ayni-orchestrator-dev".to_owned(),
            "local/ayni-orchestrator-worker:test".to_owned(),
            3,
        )?;
        assert_eq!(configuration.bind_address().to_string(), "127.0.0.1:8080");
        assert_eq!(
            configuration.kubernetes_namespace(),
            "ayni-orchestrator-dev"
        );
        assert_eq!(configuration.max_attempts(), 3);
        Ok(())
    }
    #[tokio::test]
    async fn readiness_is_explicit_about_durable_orchestration() -> Result<(), axum::Error> {
        let request = Request::builder()
            .uri("/readyz")
            .body(Body::empty())
            .map_err(axum::Error::new)?;
        let response = match router(state().map_err(axum::Error::new)?)
            .oneshot(request)
            .await
        {
            Ok(response) => response,
            Err(never) => match never {},
        };
        let body = response.into_body().collect().await?.to_bytes();
        assert_eq!(
            body,
            r#"{"component":"control-plane","status":"ready","managed_execution":true,"durable_orchestration":true,"task_ui":"available","repository_preparation":"unconfigured","pilot_ready":false,"domain_model_revision":2}"#
        );
        Ok(())
    }

    #[tokio::test]
    async fn development_api_snapshots_configuration_and_reports_unsupported_provider()
    -> Result<(), Box<dyn std::error::Error>> {
        let api = router(state()?);
        let binding = serde_json::json!({"id":"trial","owner":"ayni-oss",
            "repository":"sandbox","installation_id":1,
            "base_branch":"main",
            "lock_sha256":"d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7"});
        let profile = serde_json::json!({"id":"other","provider":"anthropic",
            "model":"example","secret_ref":"other-key"});
        for (path, payload) in [
            ("/v1/repository-bindings", binding),
            ("/v1/agent-profiles", profile),
        ] {
            let response = api.clone().oneshot(json_request(path, &payload)?).await?;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
        }
        let payload = serde_json::json!({"idempotency_key":"api-trial",
            "repository_id":"trial","profile_id":"other",
            "start_revision":"8a555ada2f6e3ee41de2485ed13da508beaebaeb",
            "request":"Correct the retry formula",
            "acceptance_criteria":["3 -> 8"]});
        let response = api
            .clone()
            .oneshot(json_request("/v1/development-tasks", &payload)?)
            .await?;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await?.to_bytes())?;
        assert_eq!(body["phase"], "unsupported");
        let task_id = body["id"].as_str().ok_or("task has no id")?;
        let response = api
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/development-tasks/{task_id}"))
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let duplicate = api
            .oneshot(json_request("/v1/development-tasks", &payload)?)
            .await?;
        assert_eq!(duplicate.status(), StatusCode::ACCEPTED);
        Ok(())
    }

    fn json_request(
        path: &str,
        value: &serde_json::Value,
    ) -> Result<Request<Body>, axum::http::Error> {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
    }
}
