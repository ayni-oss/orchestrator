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
            pilot_ready: false,
            domain_model_revision: ayni_orchestrator_domain::model_revision(),
        }
    }

    pub const fn ready(publisher_available: bool) -> Self {
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
            pilot_ready: publisher_available,
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
    #[error("evidence unavailable: {0}")]
    EvidenceUnavailable(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::InvalidIdentifier(_)
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
            Self::PublisherUnavailable => StatusCode::SERVICE_UNAVAILABLE,
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
