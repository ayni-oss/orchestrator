//! Authenticated bounded receipt, separate from controller acceptance.
use tracing::Instrument;
mod auth;
use crate::{AppState, adapters::sqlite::CleanStoreError};
use auth::KubernetesAuthority;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use ayni_orchestrator_runtime::verification::{ReportingConfiguration, ReportingReceipt};

#[derive(Clone)]
struct ReportingState {
    state: AppState,
    authority: KubernetesAuthority,
    capacity: std::sync::Arc<tokio::sync::Semaphore>,
}

#[derive(Debug)]
pub(super) enum ReceiptError {
    Invalid,
    Unauthorized,
    Conflict,
    Unavailable,
}
impl IntoResponse for ReceiptError {
    fn into_response(self) -> Response {
        match self {
            Self::Invalid => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Conflict => StatusCode::CONFLICT,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        }
        .into_response()
    }
}

/// Build the internal receipt endpoint using only in-cluster authentication.
/// # Errors
/// Returns an error when Kubernetes workload authentication is unavailable.
pub fn router(
    state: AppState,
    namespace: String,
    _configuration: &ReportingConfiguration,
) -> Result<Router, &'static str> {
    let config = kube::Config::incluster().map_err(|_| "kubernetes_incluster_required")?;
    let authority = KubernetesAuthority {
        client: kube::Client::try_from(config).map_err(|_| "kubernetes_unavailable")?,
        namespace,
    };
    Ok(Router::new()
        .route("/events/verification", post(receive))
        .layer(DefaultBodyLimit::max(
            ayni_orchestrator_events::MAX_EVENT_BYTES,
        ))
        .with_state(ReportingState {
            state,
            authority,
            capacity: std::sync::Arc::new(tokio::sync::Semaphore::new(32)),
        }))
}

async fn receive(
    State(state): State<ReportingState>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<impl IntoResponse, ReceiptError> {
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        != Some("application/cloudevents+json")
    {
        return Err(ReceiptError::Invalid);
    }
    let context = ayni_orchestrator_observability::event_context(&bytes);
    let span = ayni_orchestrator_observability::verification_span("receipt", "", context.as_ref());
    async {
        let _permit = state
            .capacity
            .try_acquire()
            .map_err(|_| ReceiptError::Unavailable)?;
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .filter(|token| !token.is_empty() && token.len() <= 8192)
            .ok_or(ReceiptError::Unauthorized)?
            .to_owned();
        let (identity, payload) =
            ayni_orchestrator_events::decode(&bytes).map_err(|_| ReceiptError::Invalid)?;
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            state.authority.authorize(token, &identity, &payload),
        )
        .await
        .map_err(|_| ReceiptError::Unavailable)??;
        let receipt = ReportingReceipt {
            source: identity.source.clone(),
            id: identity.id.clone(),
            status: "stored".into(),
        };
        state
            .state
            .database
            .execute(move |store| {
                let result = store.receive_verification(&identity, &payload);
                if result.is_ok()
                    && let Some(context) = context
                    && let Some(binding) = &payload.binding
                {
                    store.retain_verification_trace(&binding.execution_id, &context);
                }
                result
            })
            .await
            .map_err(|error| match error {
                CleanStoreError::IdempotencyConflict => ReceiptError::Conflict,
                _ => ReceiptError::Unavailable,
            })?;
        Ok((StatusCode::ACCEPTED, Json(receipt)))
    }
    .instrument(span)
    .await
}

#[cfg(test)]
mod tests;
