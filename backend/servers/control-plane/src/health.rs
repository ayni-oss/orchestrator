use axum::{
    Router,
    extract::State,
    http::{StatusCode, header},
    routing::get,
};
use ayni_orchestrator_workflow_postgres::{WorkflowStore, metrics};
use std::{fmt::Write as _, sync::Arc};

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

    async fn outcome(&self) -> Result<(), &'static str> {
        ayni_orchestrator_workflow_postgres::validate_for_runtime(
            &self.database_url,
            &self.runtime_role,
        )
        .await
        .map_err(|_| "workflow_database_unavailable")?;
        let health = WorkflowStore::new(self.database_url.clone())
            .transport_health()
            .await
            .map_err(|_| "workflow_database_unavailable")?;
        if health.full {
            return Err("workflow_report_quarantine_full");
        }
        if health.unsupported > 0 {
            return Err("workflow_report_quarantine_unsupported_version");
        }
        Ok(())
    }
}

pub fn router(readiness: WorkflowReadiness) -> Router {
    Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/readyz", get(ready))
        .route("/metrics", get(operational_metrics))
        .with_state(readiness)
}

async fn ready(State(readiness): State<WorkflowReadiness>) -> (StatusCode, &'static str) {
    match readiness.outcome().await {
        Ok(()) => (StatusCode::OK, "ready"),
        Err(reason) => (StatusCode::SERVICE_UNAVAILABLE, reason),
    }
}

async fn operational_metrics(
    State(readiness): State<WorkflowReadiness>,
) -> impl axum::response::IntoResponse {
    let mut text = metrics::render();
    let store = WorkflowStore::new(readiness.database_url.clone());
    let (status, durable) = store.operational_metrics().await.map_or_else(
        |_| (StatusCode::SERVICE_UNAVAILABLE, String::new()),
        |durable| (StatusCode::OK, durable),
    );
    text.push_str(&durable);
    let _ = writeln!(
        text,
        "workflow_database_ready {}",
        u8::from(status == StatusCode::OK)
    );
    if let Ok(health) = store.transport_health().await {
        let _ = writeln!(
            text,
            "workflow_report_quarantine_records {}",
            health.records
        );
        let _ = writeln!(text, "workflow_report_quarantine_bytes {}", health.bytes);
        let _ = writeln!(
            text,
            "workflow_report_quarantine_unsupported {}",
            health.unsupported
        );
        let _ = writeln!(
            text,
            "workflow_report_quarantine_full {}",
            u8::from(health.full)
        );
    }
    (
        status,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        text,
    )
}
