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
            task_ui: "separate",
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
            task_ui: "separate",
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
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("durable store lock was poisoned")]
    StoreLockPoisoned,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "operational endpoint unavailable".to_owned(),
            }),
        )
            .into_response()
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}
