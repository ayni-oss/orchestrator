use crate::orchestration::StoreError;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

#[derive(Serialize)]
pub struct HealthResponse {
    component: &'static str,
    status: &'static str,
    managed_execution: bool,
    durable_orchestration: bool,
    task_ui: &'static str,
    repository_preparation: &'static str,
    model_catalog: &'static str,
    pilot_ready: bool,
    domain_model_revision: u8,
}

impl HealthResponse {
    pub const fn live() -> Self {
        Self {
            component: "control-plane",
            status: "live",
            managed_execution: true,
            durable_orchestration: true,
            task_ui: "available",
            repository_preparation: "unconfigured",
            model_catalog: "unconfigured",
            pilot_ready: false,
            domain_model_revision: ayni_orchestrator_domain::model_revision(),
        }
    }

    pub const fn ready(
        publisher_available: bool,
        model_catalog_configured: bool,
        model_catalog_available: bool,
    ) -> Self {
        Self {
            component: "control-plane",
            status: "ready",
            managed_execution: true,
            durable_orchestration: true,
            task_ui: "available",
            repository_preparation: if publisher_available {
                "available"
            } else {
                "unconfigured"
            },
            model_catalog: if model_catalog_available {
                "available"
            } else if model_catalog_configured {
                "unavailable"
            } else {
                "unconfigured"
            },
            pilot_ready: publisher_available && model_catalog_available,
            domain_model_revision: ayni_orchestrator_domain::model_revision(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("invalid identifier")]
    InvalidIdentifier(#[source] ayni_orchestrator_domain::LifecycleError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("durable store lock was poisoned")]
    StoreLockPoisoned,
    #[error("pilot evidence service is unavailable")]
    PublisherUnavailable,
    #[error("provider model catalog is unavailable")]
    ModelCatalogUnavailable,
    #[error(transparent)]
    ModelCatalog(#[from] crate::ModelCatalogError),
    #[error("the selected model is not available from the provider")]
    ModelUnavailable,
    #[error("evidence unavailable: {0}")]
    EvidenceUnavailable(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::InvalidIdentifier(_)
            | Self::ModelUnavailable
            | Self::Store(
                StoreError::InvalidIdentity(_)
                | StoreError::InvalidInput(_)
                | StoreError::InvalidChangeConstraints(_)
                | StoreError::RetryNotAllowed(_)
                | StoreError::MaxAttemptsReached { .. }
                | StoreError::TaskCancelled(_)
                | StoreError::ActionKeyReused(_),
            ) => StatusCode::BAD_REQUEST,
            Self::Store(StoreError::TaskNotFound(_) | StoreError::AttemptNotFound(_)) => {
                StatusCode::NOT_FOUND
            }
            Self::EvidenceUnavailable(_) => StatusCode::NOT_FOUND,
            Self::PublisherUnavailable | Self::ModelCatalogUnavailable => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            Self::ModelCatalog(_) => StatusCode::BAD_GATEWAY,
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
