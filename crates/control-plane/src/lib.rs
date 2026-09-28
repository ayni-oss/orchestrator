//! Native gRPC, operational HTTP, and durable orchestration for Ayni Orchestrator.

mod api;
pub(crate) mod application;
mod application_metrics;
mod development;
mod development_jobs;
mod development_lifecycle;
mod github;
mod grpc;
mod grpc_access;
mod model_catalog;
mod model_usage;
mod orchestration;
mod preparation;
mod preparation_jobs;
mod publication;

use api::{ApiError, HealthResponse};
use axum::{
    Json, Router,
    extract::{MatchedPath, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use orchestration::{Store, StoreError, StoreHandle};
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
pub use grpc::{serve as serve_grpc, serve_custom as serve_custom_grpc};
pub use grpc_access::{AccessConfigurationError, CustomClientAccess};
pub use model_catalog::{AvailableModel, ModelCatalog, ModelCatalogError};
pub use orchestration::{JobTransport, KubernetesJobTransport, Reconciler, TransportError};
pub use preparation_jobs::PreparationReconciler;
pub use publication::PublisherState;

/// Configuration validated before the control plane starts accepting traffic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Configuration {
    bind_address: SocketAddr,
    grpc_bind_address: SocketAddr,
    custom_grpc: Option<CustomGrpcConfiguration>,
    database_path: PathBuf,
    kubernetes_namespace: String,
    worker_image: String,
    max_attempts: u32,
}

/// Optional TLS and identity configuration for the non-loopback custom-client listener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CustomGrpcConfiguration {
    bind_address: SocketAddr,
    certificate_path: PathBuf,
    private_key_path: PathBuf,
    clients_path: PathBuf,
}

impl CustomGrpcConfiguration {
    /// Returns the protected listener address.
    #[must_use]
    pub const fn bind_address(&self) -> SocketAddr {
        self.bind_address
    }

    /// Loads TLS material and credential digests without exposing their contents.
    ///
    /// # Errors
    ///
    /// Returns an error when a configured file cannot be read or validated.
    pub fn load_access(&self) -> Result<CustomClientAccess, AccessConfigurationError> {
        CustomClientAccess::load(
            &self.certificate_path,
            &self.private_key_path,
            &self.clients_path,
        )
    }
}

impl Configuration {
    /// The default address used when no address is configured.
    pub const DEFAULT_BIND_ADDRESS: &'static str = "0.0.0.0:8080";
    /// Default native gRPC listener used by the trusted local profile.
    pub const DEFAULT_GRPC_BIND_ADDRESS: &'static str = "127.0.0.1:50051";
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
        let grpc_bind_address = env::var("ORCHESTRATOR_GRPC_BIND_ADDRESS")
            .unwrap_or_else(|_| Self::DEFAULT_GRPC_BIND_ADDRESS.to_owned());
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
        let mut configuration = Self::from_values(
            &bind_address,
            &grpc_bind_address,
            PathBuf::from(database_path),
            kubernetes_namespace,
            worker_image,
            max_attempts,
        )?;
        configuration.custom_grpc = custom_grpc_from_environment(
            configuration.bind_address,
            configuration.grpc_bind_address,
        )?;
        Ok(configuration)
    }

    /// Validates explicit configuration values.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigurationError`] for invalid socket or namespace values.
    pub fn from_values(
        bind_address: &str,
        grpc_bind_address: &str,
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
        let grpc_bind_address = SocketAddr::from_str(grpc_bind_address).map_err(|source| {
            ConfigurationError::InvalidGrpcBindAddress {
                value: grpc_bind_address.to_owned(),
                source,
            }
        })?;
        if grpc_bind_address == bind_address {
            return Err(ConfigurationError::DuplicateBindAddress(bind_address));
        }
        if !grpc_bind_address.ip().is_loopback() {
            return Err(ConfigurationError::NonLoopbackGrpcBindAddress(
                grpc_bind_address,
            ));
        }
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
            grpc_bind_address,
            custom_grpc: None,
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
    /// Returns the native gRPC listener address.
    #[must_use]
    pub const fn grpc_bind_address(&self) -> SocketAddr {
        self.grpc_bind_address
    }
    /// Returns the optional non-loopback custom-client listener configuration.
    #[must_use]
    pub const fn custom_grpc(&self) -> Option<&CustomGrpcConfiguration> {
        self.custom_grpc.as_ref()
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

fn custom_grpc_from_environment(
    http_address: SocketAddr,
    local_grpc_address: SocketAddr,
) -> Result<Option<CustomGrpcConfiguration>, ConfigurationError> {
    const KEYS: [&str; 4] = [
        "ORCHESTRATOR_CUSTOM_GRPC_BIND_ADDRESS",
        "ORCHESTRATOR_CUSTOM_GRPC_TLS_CERTIFICATE_PATH",
        "ORCHESTRATOR_CUSTOM_GRPC_TLS_PRIVATE_KEY_PATH",
        "ORCHESTRATOR_CUSTOM_GRPC_CLIENTS_PATH",
    ];
    custom_grpc_from_values(
        KEYS.map(|key| env::var(key).ok()),
        http_address,
        local_grpc_address,
    )
}

fn custom_grpc_from_values(
    values: [Option<String>; 4],
    http_address: SocketAddr,
    local_grpc_address: SocketAddr,
) -> Result<Option<CustomGrpcConfiguration>, ConfigurationError> {
    if values.iter().all(Option::is_none) {
        return Ok(None);
    }
    let [
        Some(address),
        Some(certificate),
        Some(private_key),
        Some(clients),
    ] = values
    else {
        return Err(ConfigurationError::IncompleteCustomGrpcConfiguration);
    };
    let bind_address = SocketAddr::from_str(&address).map_err(|source| {
        ConfigurationError::InvalidCustomGrpcBindAddress {
            value: address,
            source,
        }
    })?;
    if bind_address.ip().is_loopback()
        || bind_address.port() == 0
        || bind_address.port() == http_address.port()
        || bind_address.port() == local_grpc_address.port()
    {
        return Err(ConfigurationError::UnsafeCustomGrpcBindAddress(
            bind_address,
        ));
    }
    let certificate_path = PathBuf::from(certificate);
    let private_key_path = PathBuf::from(private_key);
    let clients_path = PathBuf::from(clients);
    if [&certificate_path, &private_key_path, &clients_path]
        .iter()
        .any(|path| !path.is_absolute() || path.as_os_str().is_empty())
    {
        return Err(ConfigurationError::InvalidCustomGrpcPath);
    }
    Ok(Some(CustomGrpcConfiguration {
        bind_address,
        certificate_path,
        private_key_path,
        clients_path,
    }))
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
    /// `ORCHESTRATOR_GRPC_BIND_ADDRESS` was not a valid address.
    #[error("ORCHESTRATOR_GRPC_BIND_ADDRESS must be a socket address, got {value:?}")]
    InvalidGrpcBindAddress {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
    /// HTTP and gRPC cannot listen on the same socket.
    #[error("HTTP and gRPC bind addresses must differ, both were {0}")]
    DuplicateBindAddress(SocketAddr),
    /// The unauthenticated product API must remain reachable only through pod loopback.
    #[error("ORCHESTRATOR_GRPC_BIND_ADDRESS must use a loopback address, got {0}")]
    NonLoopbackGrpcBindAddress(SocketAddr),
    /// Only some protected listener settings were supplied.
    #[error("all ORCHESTRATOR_CUSTOM_GRPC_* settings are required together")]
    IncompleteCustomGrpcConfiguration,
    /// The protected listener address was malformed.
    #[error("ORCHESTRATOR_CUSTOM_GRPC_BIND_ADDRESS must be a socket address, got {value:?}")]
    InvalidCustomGrpcBindAddress {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
    /// The protected listener did not have a separate non-loopback address.
    #[error(
        "ORCHESTRATOR_CUSTOM_GRPC_BIND_ADDRESS must be a distinct non-loopback address with a nonzero port, got {0}"
    )]
    UnsafeCustomGrpcBindAddress(SocketAddr),
    /// Protected listener file paths must be absolute.
    #[error("custom-client TLS and identity configuration paths must be absolute")]
    InvalidCustomGrpcPath,
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
        .route("/livez", get(liveness))
        .route("/readyz", get(readiness))
        .route("/metrics", get(metrics))
        .route(
            "/internal/publications/{task_id}",
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

async fn liveness() -> impl IntoResponse {
    (StatusCode::OK, Json(HealthResponse::live()))
}
async fn metrics(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let body = with_store(&state, |store| store.application_metrics())?;
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

pub(crate) fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_owned()
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
            "127.0.0.1:50051",
            PathBuf::from("/data/tasks.sqlite"),
            "ayni-orchestrator-dev".to_owned(),
            "local/ayni-orchestrator-worker:test".to_owned(),
            3,
        )?;
        assert_eq!(configuration.bind_address().to_string(), "127.0.0.1:8080");
        assert_eq!(
            configuration.grpc_bind_address().to_string(),
            "127.0.0.1:50051"
        );
        assert_eq!(
            configuration.kubernetes_namespace(),
            "ayni-orchestrator-dev"
        );
        assert_eq!(configuration.max_attempts(), 3);
        Ok(())
    }

    #[test]
    fn rejects_non_loopback_grpc_configuration() {
        let Err(error) = Configuration::from_values(
            "0.0.0.0:8080",
            "0.0.0.0:50051",
            PathBuf::from("/data/tasks.sqlite"),
            "ayni-orchestrator-dev".to_owned(),
            "local/ayni-orchestrator-worker:test".to_owned(),
            3,
        ) else {
            panic!("an unauthenticated non-loopback gRPC listener was accepted");
        };
        assert!(matches!(
            error,
            ConfigurationError::NonLoopbackGrpcBindAddress(_)
        ));
    }

    #[test]
    fn protected_grpc_configuration_is_all_or_nothing_and_non_loopback() {
        let http = SocketAddr::from(([0, 0, 0, 0], 8080));
        let local = SocketAddr::from(([127, 0, 0, 1], 50051));
        let incomplete = custom_grpc_from_values(
            [Some("0.0.0.0:50052".to_owned()), None, None, None],
            http,
            local,
        );
        assert!(matches!(
            incomplete,
            Err(ConfigurationError::IncompleteCustomGrpcConfiguration)
        ));
        let ephemeral = custom_grpc_from_values(
            [
                Some("0.0.0.0:0".to_owned()),
                Some("/tls.crt".to_owned()),
                Some("/tls.key".to_owned()),
                Some("/clients.json".to_owned()),
            ],
            http,
            local,
        );
        assert!(matches!(
            ephemeral,
            Err(ConfigurationError::UnsafeCustomGrpcBindAddress(_))
        ));
        let configured = custom_grpc_from_values(
            [
                Some("0.0.0.0:50052".to_owned()),
                Some("/tls.crt".to_owned()),
                Some("/tls.key".to_owned()),
                Some("/clients.json".to_owned()),
            ],
            http,
            local,
        );
        assert_eq!(
            configured.ok().flatten().map(|value| value.bind_address()),
            Some(SocketAddr::from(([0, 0, 0, 0], 50052)))
        );
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
            r#"{"component":"control-plane","status":"ready","managed_execution":true,"durable_orchestration":true,"task_ui":"separate","repository_preparation":"unconfigured","model_catalog":"available","pilot_ready":false,"domain_model_revision":4}"#
        );
        Ok(())
    }

    #[tokio::test]
    async fn missing_model_catalog_changes_readiness_without_removing_operations()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut unavailable = state()?;
        unavailable.model_catalog = None;
        let response = router(unavailable)
            .oneshot(Request::builder().uri("/readyz").body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await?.to_bytes())?;
        assert_eq!(body["model_catalog"], "unconfigured");
        assert_eq!(body["pilot_ready"], false);
        Ok(())
    }

    #[tokio::test]
    async fn operational_http_does_not_serve_browser_or_product_routes()
    -> Result<(), Box<dyn std::error::Error>> {
        let api = router(state()?);
        for path in ["/", "/v1/provider-models", "/v1/development-tasks"] {
            let response = api
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty())?)
                .await?;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn internal_publication_callback_remains_routed_and_fail_closed()
    -> Result<(), Box<dyn std::error::Error>> {
        let response = router(state()?)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/publications/00000000-0000-4000-8000-000000000001")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"title":"fix: bounded change","body":"summary"}"#,
                    ))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        Ok(())
    }
}
