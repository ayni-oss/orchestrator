//! HTTP API and durable orchestration surface for Ayni Orchestrator.

mod development;
mod development_jobs;
mod github;
mod orchestration;
mod publication;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use ayni_orchestrator_domain::{AttemptId, AttemptOutcome, IdempotencyKey, TaskId};
use development::{AgentProfile, DevelopmentInput, DevelopmentTask, RepositoryBinding};
use orchestration::{Store, StoreError, StoreHandle, TaskInput, TaskView};
use serde::{Deserialize, Serialize};
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
        .route("/livez", get(liveness))
        .route("/readyz", get(readiness))
        .route("/v1/tasks", post(submit_task))
        .route("/v1/tasks/{task_id}", get(task))
        .route("/v1/tasks/{task_id}/retry", post(retry_task))
        .route("/v1/tasks/{task_id}/cancel", post(cancel_task))
        .route("/v1/attempts/{attempt_id}/reports", post(report_attempt))
        .route("/v1/repository-bindings", post(put_repository_binding))
        .route("/v1/agent-profiles", post(put_agent_profile))
        .route("/v1/development-tasks", post(submit_development_task))
        .route("/v1/development-tasks/{task_id}", get(development_task))
        .route(
            "/v1/development-tasks/{task_id}/cancel",
            post(cancel_development_task),
        )
        .route(
            "/v1/development-tasks/{task_id}/publish",
            post(publication::publish),
        )
        .with_state(state)
}

async fn put_repository_binding(
    State(state): State<AppState>,
    Json(binding): Json<RepositoryBinding>,
) -> Result<StatusCode, ApiError> {
    with_store(&state, |store| store.put_repository_binding(&binding))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_agent_profile(
    State(state): State<AppState>,
    Json(profile): Json<AgentProfile>,
) -> Result<StatusCode, ApiError> {
    with_store(&state, |store| store.put_agent_profile(&profile))?;
    Ok(StatusCode::NO_CONTENT)
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

async fn liveness() -> impl IntoResponse {
    (StatusCode::OK, Json(HealthResponse::live()))
}
async fn readiness() -> impl IntoResponse {
    (StatusCode::OK, Json(HealthResponse::ready()))
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
struct ActionRequest {
    idempotency_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportRequest {
    idempotency_key: String,
    outcome: AttemptOutcome,
}

#[derive(Serialize)]
struct HealthResponse {
    component: &'static str,
    status: &'static str,
    managed_execution: bool,
    durable_orchestration: bool,
    domain_model_revision: u8,
}
impl HealthResponse {
    const fn live() -> Self {
        Self {
            component: "control-plane",
            status: "live",
            managed_execution: true,
            durable_orchestration: true,
            domain_model_revision: ayni_orchestrator_domain::model_revision(),
        }
    }
    const fn ready() -> Self {
        Self {
            component: "control-plane",
            status: "ready",
            managed_execution: true,
            durable_orchestration: true,
            domain_model_revision: ayni_orchestrator_domain::model_revision(),
        }
    }
}

#[derive(Debug, Error)]
enum ApiError {
    #[error("invalid identifier")]
    InvalidIdentifier(#[source] ayni_orchestrator_domain::LifecycleError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("durable store lock was poisoned")]
    StoreLockPoisoned,
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::InvalidIdentifier(_)
            | Self::Store(
                StoreError::InvalidIdentity(_)
                | StoreError::InvalidInput(_)
                | StoreError::RetryNotAllowed(_)
                | StoreError::MaxAttemptsReached { .. }
                | StoreError::TaskCancelled(_)
                | StoreError::ActionKeyReused(_),
            ) => StatusCode::BAD_REQUEST,
            Self::Store(StoreError::TaskNotFound(_) | StoreError::AttemptNotFound(_)) => {
                StatusCode::NOT_FOUND
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(ErrorResponse {
                error: self.to_string(),
            }),
        )
            .into_response()
    }
}
#[derive(Serialize)]
struct ErrorResponse {
    error: String,
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
            r#"{"component":"control-plane","status":"ready","managed_execution":true,"durable_orchestration":true,"domain_model_revision":2}"#
        );
        Ok(())
    }

    #[tokio::test]
    async fn development_api_snapshots_configuration_and_reports_unsupported_provider()
    -> Result<(), Box<dyn std::error::Error>> {
        let api = router(state()?);
        let binding = serde_json::json!({"id":"trial","owner":"ayni-oss",
            "repository":"orchestrator","installation_id":1,
            "base_branch":"feat/request-to-draft-pr",
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
