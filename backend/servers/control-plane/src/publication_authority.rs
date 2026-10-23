//! Repository-bound publication authority cannot be used by coding workers.
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use ayni_orchestrator_events::task_publication::{
    PublicationRun, PublicationSupervisorRequest, PublicationWorkerRequest,
    PublicationWorkerResponse, github_repository,
};
use ayni_orchestrator_workflow_postgres::WorkflowStore;
use sha2::{Digest as _, Sha256};
use std::{io, sync::Arc};
use subtle::ConstantTimeEq as _;
#[derive(Clone)]
struct Authority {
    store: WorkflowStore,
    token: Arc<str>,
    workspace: Arc<str>,
    repository: Arc<str>,
}
/// Install publication only when a separate configured credential and binding exist.
/// # Errors
/// Rejects incomplete configuration or invalid repository identity.
pub fn router(store: WorkflowStore) -> Result<Router, io::Error> {
    let path = match std::env::var("ORCHESTRATOR_PUBLICATION_AUTHORITY_TOKEN_PATH") {
        Ok(v) => v,
        Err(std::env::VarError::NotPresent) => return Ok(Router::new()),
        Err(_) => return Err(io::Error::other("publication configuration invalid")),
    };
    let token = std::fs::read_to_string(path)?;
    let token = token.trim();
    if !(32..=256).contains(&token.len()) {
        return Err(io::Error::other("publication authority invalid"));
    }
    let workspace = std::env::var("ORCHESTRATOR_PUBLICATION_WORKSPACE_ID")
        .map_err(|_| io::Error::other("publication workspace missing"))?;
    uuid::Uuid::parse_str(&workspace)
        .map_err(|_| io::Error::other("publication workspace invalid"))?;
    let repository = std::env::var("ORCHESTRATOR_PUBLICATION_REPOSITORY")
        .ok()
        .and_then(|s| github_repository(&s))
        .ok_or_else(|| io::Error::other("publication repository invalid"))?;
    Ok(Router::new()
        .route("/v1/publication-supervisor", post(poll))
        .route(
            "/v1/publication-worker/{publication}/{generation}",
            post(worker),
        )
        .layer(DefaultBodyLimit::max(32 * 1024))
        .with_state(Authority {
            store,
            token: Arc::from(token),
            workspace: Arc::from(workspace),
            repository: Arc::from(repository),
        }))
}
fn authorized(headers: &HeaderMap, token: &str) -> Result<(), StatusCode> {
    let value = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if bool::from(Sha256::digest(value.as_bytes()).ct_eq(&Sha256::digest(token.as_bytes()))) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
async fn poll(
    State(s): State<Authority>,
    headers: HeaderMap,
    Json(request): Json<PublicationSupervisorRequest>,
) -> Result<Json<Option<PublicationRun>>, StatusCode> {
    authorized(&headers, &s.token)?;
    s.store
        .publication_supervisor(&s.workspace, &s.repository, &request)
        .await
        .map(Json)
        .map_err(|_| StatusCode::CONFLICT)
}
async fn worker(
    State(s): State<Authority>,
    Path((id, generation)): Path<(String, u64)>,
    headers: HeaderMap,
    Json(request): Json<PublicationWorkerRequest>,
) -> Result<Json<PublicationWorkerResponse>, StatusCode> {
    authorized(&headers, &s.token)?;
    let uid = headers
        .get("x-publication-pod-uid")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    s.store
        .publication_worker(
            (&s.workspace, &s.repository),
            &id,
            generation,
            uid,
            &request,
        )
        .await
        .map(Json)
        .map_err(|_| StatusCode::CONFLICT)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn other_capabilities_cannot_publish() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            "Bearer task-worker-token"
                .parse()
                .unwrap_or_else(|_| panic!("fixture")),
        );
        assert!(authorized(&headers, "publication-authority-token").is_err());
    }
}
