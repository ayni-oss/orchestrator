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
use ayni_orchestrator_workflow_postgres::WorkflowStore;
use sha2::{Digest as _, Sha256};
use std::{io, sync::Arc};
use subtle::ConstantTimeEq as _;
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
        .layer(DefaultBodyLimit::max(256 * 1024))
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
    s.store
        .task_worker(&attempt, generation, uid, &request)
        .await
        .map(Json)
        .map_err(|_| StatusCode::CONFLICT)
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
