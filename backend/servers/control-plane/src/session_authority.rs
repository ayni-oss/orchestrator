//! Only the trusted supervisor can hydrate frozen context and admit a paid call.
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use ayni_orchestrator_observability::{
    event_context,
    workflow::{Workflow, finish},
};
use ayni_orchestrator_workflow_postgres::{
    WorkflowStore,
    store::provider_calls::{AuthorityRequest, AuthorityResponse},
};
use sha2::{Digest as _, Sha256};
use std::{io, sync::Arc};
use subtle::ConstantTimeEq as _;
use tracing::Instrument as _;

#[derive(Clone)]
struct Authority {
    store: WorkflowStore,
    token: Arc<str>,
}

/// Install the supervisor boundary only when its mounted token is configured.
///
/// # Errors
/// A configured but unavailable token fails startup closed.
pub fn router(store: WorkflowStore) -> Result<Router, io::Error> {
    let path = match std::env::var("ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH") {
        Ok(path) => path,
        Err(std::env::VarError::NotPresent) => return Ok(Router::new()),
        Err(_) => return Err(io::Error::other("authority token path invalid")),
    };
    let token = std::fs::read_to_string(path)
        .map_err(|_| io::Error::other("authority token unavailable"))?;
    let token = token.trim().to_owned();
    if token.len() < 32 || token.len() > 256 {
        return Err(io::Error::other("authority token invalid"));
    }
    Ok(Router::new()
        .route("/v1/session-authority", post(authority))
        .route("/v1/session-credential", post(credential))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(Authority {
            store,
            token: token.into(),
        }))
}

async fn authority(
    State(state): State<Authority>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::Json<AuthorityResponse>, StatusCode> {
    authenticate(&state, &headers)?;
    let request: AuthorityRequest =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let diagnostic = event_context(&request.command_bytes);
    let workflow = Workflow::from_event(&request.command_bytes);
    let operation = match request.action.as_str() {
        "admit" => "provider.authority.admit",
        "progress" => "provider.authority.progress",
        "poll" => "provider.authority.poll",
        _ => "provider.authority.context",
    };
    let span = workflow.span_with_context(operation, diagnostic.as_ref(), false);
    let started = std::time::Instant::now();
    let result = state
        .store
        .session_authority(&request)
        .instrument(span.clone())
        .await;
    ayni_orchestrator_workflow_postgres::metrics::observe(
        "workflow_provider_authority_seconds",
        started.elapsed(),
    );
    ayni_orchestrator_workflow_postgres::metrics::increment(if result.is_ok() {
        "workflow_provider_authority_requests_total"
    } else {
        "workflow_provider_authority_failures_total"
    });
    finish(&span, result.is_ok());
    result.map(axum::Json).map_err(|_| StatusCode::CONFLICT)
}

fn authenticate(state: &Authority, headers: &HeaderMap) -> Result<(), StatusCode> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let provided = Sha256::digest(token.as_bytes());
    let expected = Sha256::digest(state.token.as_bytes());
    if !bool::from(provided.as_slice().ct_eq(expected.as_slice())) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}
async fn credential(
    State(state): State<Authority>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<([(axum::http::header::HeaderName, &'static str); 1], String), StatusCode> {
    authenticate(&state, &headers)?;
    let request: AuthorityRequest =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let value = state
        .store
        .session_credential(request.command_bytes)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    Ok(([(axum::http::header::CACHE_CONTROL, "no-store")], value))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn authority_rejects_missing_or_wrong_bearer_before_parsing_commands() {
        let state = Authority {
            store: WorkflowStore::new("not-a-database"),
            token: Arc::from("fixture-supervisor-token-01234567890123456789"),
        };
        for token in [None, Some("Bearer wrong-token")] {
            let mut headers = HeaderMap::new();
            if let Some(token) = token {
                headers.insert(
                    "authorization",
                    token.parse().unwrap_or_else(|_| panic!("fixture header")),
                );
            }
            assert!(matches!(
                authority(
                    State(state.clone()),
                    headers,
                    Bytes::from_static(b"malformed")
                )
                .await,
                Err(StatusCode::UNAUTHORIZED)
            ));
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            "Bearer fixture-supervisor-token-01234567890123456789"
                .parse()
                .unwrap_or_else(|_| panic!("fixture header")),
        );
        assert!(matches!(
            authority(State(state), headers, Bytes::from_static(b"malformed")).await,
            Err(StatusCode::BAD_REQUEST)
        ));
    }
}
