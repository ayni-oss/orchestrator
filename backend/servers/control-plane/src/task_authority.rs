//! Scoped worker capabilities are distinct from the trusted supervisor bearer.
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use ayni_orchestrator_events::task_execution::{
    SupervisorRequest, TaskRun, WorkerRequest, WorkerResponse, worker_token,
};
use ayni_orchestrator_observability::{
    TraceContext,
    workflow::{Workflow, finish, identity_context},
};
use ayni_orchestrator_workflow_postgres::WorkflowStore;
use sha2::{Digest as _, Sha256};
use std::{io, sync::Arc};
use subtle::ConstantTimeEq as _;
use tracing::Instrument as _;
#[derive(Clone)]
struct Authority {
    store: WorkflowStore,
    token: Arc<str>,
}
/// Install authenticated task authority when the existing supervisor capability is mounted.
/// # Errors
/// A configured but unavailable bearer fails startup closed.
pub fn router(store: WorkflowStore) -> Result<Router, io::Error> {
    let path = match std::env::var("ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH") {
        Ok(v) => v,
        Err(std::env::VarError::NotPresent) => return Ok(Router::new()),
        Err(_) => return Err(io::Error::other("task authority configuration invalid")),
    };
    let token = std::fs::read_to_string(path)
        .map_err(|_| io::Error::other("task authority unavailable"))?;
    let token = token.trim();
    if !(32..=256).contains(&token.len()) {
        return Err(io::Error::other("task authority invalid"));
    }
    Ok(Router::new()
        .route("/v1/task-supervisor", post(supervisor))
        .route("/v1/task-worker/{attempt}/{generation}", post(worker))
        .layer(DefaultBodyLimit::max(
            ayni_orchestrator_events::task_execution::MAX_TASK_WORKER_REQUEST_BYTES,
        ))
        .with_state(Authority {
            store,
            token: Arc::from(token),
        }))
}
fn authorized(headers: &HeaderMap, expected: &str) -> Result<(), StatusCode> {
    let actual = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if bool::from(Sha256::digest(actual.as_bytes()).ct_eq(&Sha256::digest(expected.as_bytes()))) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
async fn supervisor(
    State(s): State<Authority>,
    headers: HeaderMap,
    Json(request): Json<SupervisorRequest>,
) -> Result<Json<Option<TaskRun>>, StatusCode> {
    authorized(&headers, &s.token)?;
    s.store
        .task_supervisor(&request)
        .await
        .map(Json)
        .map_err(|_| StatusCode::CONFLICT)
}
async fn worker(
    State(s): State<Authority>,
    Path((attempt, generation)): Path<(String, u64)>,
    headers: HeaderMap,
    Json(request): Json<WorkerRequest>,
) -> Result<Json<WorkerResponse>, StatusCode> {
    uuid::Uuid::parse_str(&attempt).map_err(|_| StatusCode::BAD_REQUEST)?;
    if generation == 0 {
        return Err(StatusCode::BAD_REQUEST);
    }
    authorized(
        &headers,
        &worker_token(s.token.as_bytes(), &attempt, generation),
    )?;
    let uid = headers
        .get("x-task-pod-uid")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let stage = match &request {
        WorkerRequest::QualityReport { .. } => "task.quality.accept",
        WorkerRequest::Report { .. } => "task.coding.accept",
        WorkerRequest::Sending { .. } => "task.provider.admit",
        WorkerRequest::Input => "task.source.fetch",
    };
    let diagnostic = headers
        .get("traceparent")
        .and_then(|value| value.to_str().ok())
        .and_then(|parent| TraceContext::parse(parent, None))
        .or_else(|| identity_context(&attempt));
    let span = Workflow {
        attempt_id: attempt.clone(),
        generation,
        ..Workflow::default()
    }
    .span_with_context(stage, diagnostic.as_ref(), false);
    let started = std::time::Instant::now();
    let result = s
        .store
        .task_worker(&attempt, generation, uid, &request)
        .instrument(span.clone())
        .await;
    ayni_orchestrator_workflow_postgres::metrics::observe(
        "workflow_task_authority_seconds",
        started.elapsed(),
    );
    ayni_orchestrator_workflow_postgres::metrics::increment(if result.is_ok() {
        "workflow_task_authority_requests_total"
    } else {
        "workflow_task_authority_failures_total"
    });
    if let Ok(response) = &result
        && let Some(command) = &response.command
    {
        span.record("workspace_id", command.workspace_id.as_str());
        span.record("project_id", command.project_id.as_str());
        span.record("conversation_id", command.conversation_id.as_str());
        span.record("task_id", command.task_id.as_str());
        span.record("plan_id", command.plan_id.as_str());
    }
    finish(&span, result.is_ok());
    result.map(Json).map_err(|_| StatusCode::CONFLICT)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_capabilities_cannot_authorize_the_supervisor_or_another_generation() {
        let secret = "fixture-authority-01234567890123456789";
        let token = worker_token(secret.as_bytes(), "attempt", 1);
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .unwrap_or_else(|_| panic!("fixture")),
        );
        assert!(authorized(&headers, secret).is_err());
        assert!(authorized(&headers, &worker_token(secret.as_bytes(), "attempt", 2)).is_err());
        assert!(authorized(&headers, &token).is_ok());
    }
}
