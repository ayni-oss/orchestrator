use axum::{Router, extract::State, http::StatusCode, routing::get};
use std::sync::Arc;

#[derive(Clone)]
pub struct WorkflowReadiness {
    database_url: Arc<str>,
    runtime_role: Arc<str>,
}

impl WorkflowReadiness {
    #[must_use]
    pub fn new(database_url: impl Into<Arc<str>>, runtime_role: impl Into<Arc<str>>) -> Self {
        Self {
            database_url: database_url.into(),
            runtime_role: runtime_role.into(),
        }
    }

    async fn ready(&self) -> bool {
        ayni_orchestrator_workflow_postgres::validate_for_runtime(
            &self.database_url,
            &self.runtime_role,
        )
        .await
        .is_ok()
    }
}

pub fn router(readiness: WorkflowReadiness) -> Router {
    Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/readyz", get(ready))
        .with_state(readiness)
}

async fn ready(State(readiness): State<WorkflowReadiness>) -> StatusCode {
    if readiness.ready().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
