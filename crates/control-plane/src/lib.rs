//! HTTP API and durable orchestration surface for Ayni Orchestrator.

mod api;
mod development;
mod development_jobs;
mod development_lifecycle;
mod github;
mod model_catalog;
mod model_usage;
mod orchestration;
mod preparation;
mod preparation_jobs;
mod publication;
mod ui;

use api::{ApiError, HealthResponse};
use axum::{
    Json, Router,
    extract::{MatchedPath, Path, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use ayni_orchestrator_domain::{
    AttemptId, AttemptOutcome, ChangeConstraints, IdempotencyKey, TaskId,
};
use development::{DevelopmentInput, DevelopmentTask, RepositoryBinding};
use orchestration::{Store, StoreError, StoreHandle, TaskInput, TaskView};
use preparation::{PreparationView, PrepareRepositoryInput, RepositoryView};
use serde::Deserialize;
use std::{
    env,
    net::SocketAddr,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Instant,
};
use thiserror::Error;
use tracing::{Instrument, info, info_span};
use uuid::Uuid;

pub use development_jobs::DevelopmentReconciler;
pub use model_catalog::{AvailableModel, ModelCatalog, ModelCatalogError};
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
    model_catalog: Option<ModelCatalog>,
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
            model_catalog: None,
        })
    }

    #[must_use]
    pub fn with_publisher(mut self, publisher: Option<PublisherState>) -> Self {
        self.publisher = publisher.map(Arc::new);
        self
    }

    #[must_use]
    pub fn with_model_catalog(mut self, model_catalog: ModelCatalog) -> Self {
        self.model_catalog = Some(model_catalog);
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

/// Builds the control-plane HTTP router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(ui::index))
        .route("/livez", get(liveness))
        .route("/readyz", get(readiness))
        .route("/metrics", get(metrics))
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
        .route("/v1/provider-models", get(provider_models))
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
        .layer(middleware::from_fn(observe_http))
        .with_state(state)
}

async fn observe_http(request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
        .map_or_else(|| Uuid::now_v7().to_string(), str::to_owned);
    let method = request.method().to_string();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    let span = info_span!("http.request", %request_id, %method, %route);
    async move {
        let started = Instant::now();
        let mut response = next.run(request).await;
        let status = response.status().as_u16();
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        info!(status, latency_ms, "HTTP request completed");
        if let Ok(value) = HeaderValue::from_str(&request_id) {
            response.headers_mut().insert("x-request-id", value);
        }
        response
    }
    .instrument(span)
    .await
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

async fn provider_models(
    State(state): State<AppState>,
) -> Result<Json<model_catalog::AvailableModels>, ApiError> {
    let catalog = state
        .model_catalog
        .as_ref()
        .ok_or(ApiError::ModelCatalogUnavailable)?;
    Ok(Json(catalog.available().await?))
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
    let key = parse_key(request.idempotency_key.clone())?;
    let model = request.model.clone();
    let profile_id = Store::discovered_model_profile_id(&model);
    let input = request.into_input(profile_id);
    if let Some(task) = with_store(&state, |store| {
        store.existing_development_submission(&key, &input)
    })? {
        return Ok((StatusCode::ACCEPTED, Json(task)));
    }
    let catalog = state
        .model_catalog
        .as_ref()
        .ok_or(ApiError::ModelCatalogUnavailable)?;
    if !catalog.contains(&model).await? {
        return Err(ApiError::ModelUnavailable);
    }
    with_store(&state, |store| store.put_discovered_model(&model))?;
    let task = with_store(&state, |store| store.submit_development(&key, &input))?;
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
async fn metrics(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let body = with_store(&state, |store| store.model_usage_metrics())?;
    Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body))
}
async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse::ready(
            state.publisher.is_some(),
            state.model_catalog.is_some(),
            state
                .model_catalog
                .as_ref()
                .is_some_and(ModelCatalog::is_verified),
        )),
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
    repository_id: String,
    model: String,
    start_revision: String,
    request: String,
    acceptance_criteria: Vec<String>,
    #[serde(default)]
    change_constraints: ChangeConstraints,
    #[serde(default)]
    deadline_seconds: Option<u64>,
}

impl SubmitDevelopmentRequest {
    fn into_input(self, profile_id: String) -> DevelopmentInput {
        DevelopmentInput {
            repository_id: self.repository_id,
            profile_id,
            start_revision: self.start_revision,
            request: self.request,
            acceptance_criteria: self.acceptance_criteria,
            change_constraints: self.change_constraints,
            deadline_seconds: self.deadline_seconds,
        }
    }
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
            model_catalog: Some(ModelCatalog::fixed(vec![AvailableModel {
                id: "gpt-5".into(),
                name: Some("GPT-5".into()),
                owned_by: Some("openai".into()),
                created_at: None,
                context_length: None,
                max_output_tokens: None,
            }])),
        })
    }
    #[test]
    fn accepts_durable_configuration() -> Result<(), ConfigurationError> {
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
            .header("x-request-id", "readiness-test")
            .body(Body::empty())
            .map_err(axum::Error::new)?;
        let response = match router(state().map_err(axum::Error::new)?)
            .oneshot(request)
            .await
        {
            Ok(response) => response,
            Err(never) => match never {},
        };
        assert_eq!(response.headers()["x-request-id"], "readiness-test");
        let body = response.into_body().collect().await?.to_bytes();
        assert_eq!(
            body,
            r#"{"component":"control-plane","status":"ready","managed_execution":true,"durable_orchestration":true,"task_ui":"available","repository_preparation":"unconfigured","model_catalog":"available","pilot_ready":false,"domain_model_revision":4}"#
        );
        Ok(())
    }

    #[tokio::test]
    async fn missing_model_catalog_degrades_only_catalog_dependent_routes()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut unavailable = state()?;
        unavailable.model_catalog = None;
        let api = router(unavailable);
        let response = api
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/provider-models")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let response = api
            .oneshot(Request::builder().uri("/readyz").body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await?.to_bytes())?;
        assert_eq!(body["model_catalog"], "unconfigured");
        assert_eq!(body["pilot_ready"], false);

        let configured = state()?.with_model_catalog(ModelCatalog::openai("test-key")?);
        let response = router(configured)
            .oneshot(Request::builder().uri("/readyz").body(Body::empty())?)
            .await?;
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await?.to_bytes())?;
        assert_eq!(body["model_catalog"], "unavailable");
        assert_eq!(body["pilot_ready"], false);
        Ok(())
    }

    #[tokio::test]
    async fn provider_models_returns_the_sanitized_catalog()
    -> Result<(), Box<dyn std::error::Error>> {
        let response = router(state()?)
            .oneshot(
                Request::builder()
                    .uri("/v1/provider-models")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await?.to_bytes())?;
        assert_eq!(body["provider"], "openai");
        assert_eq!(body["models"][0]["id"], "gpt-5");
        assert_eq!(body["models"][0]["name"], "GPT-5");
        Ok(())
    }

    #[tokio::test]
    async fn development_api_rejects_a_model_absent_from_the_provider_catalog()
    -> Result<(), Box<dyn std::error::Error>> {
        let payload = serde_json::json!({
            "idempotency_key":"unavailable-model",
            "repository_id":"trial",
            "model":"luna",
            "start_revision":"8a555ada2f6e3ee41de2485ed13da508beaebaeb",
            "request":"Correct the retry formula",
            "acceptance_criteria":["3 -> 8"]
        });
        let response = router(state()?)
            .oneshot(json_request("/v1/development-tasks", &payload)?)
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response.into_body().collect().await?.to_bytes();
        assert!(String::from_utf8(body.to_vec())?.contains("not available"));
        Ok(())
    }

    #[tokio::test]
    async fn legacy_agent_profile_configuration_is_not_public()
    -> Result<(), Box<dyn std::error::Error>> {
        let response = router(state()?)
            .oneshot(
                Request::builder()
                    .uri("/v1/agent-profiles")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
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
