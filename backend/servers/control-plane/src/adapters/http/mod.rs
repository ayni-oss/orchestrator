//! Server-owned operational HTTP endpoints, separate from the product gRPC API.

mod health;

use crate::{AppState, adapters::models::ModelCatalog, process::metrics as application_metrics};
use axum::{
    Json, Router,
    extract::{MatchedPath, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use health::HealthResponse;
use std::{sync::Arc, time::Instant};
use tracing::{Instrument, debug, debug_span};
use uuid::Uuid;

/// Probe handlers receive only model-catalog health. They cannot access the
/// durable store or provider clients through their router state.
#[derive(Clone)]
struct HttpState {
    model_catalog_configured: bool,
    model_catalog_verified: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl From<AppState> for HttpState {
    fn from(state: AppState) -> Self {
        let catalog = state.model_catalog();
        Self {
            model_catalog_configured: catalog.is_some(),
            // Observe the shared catalog on every probe: startup can precede
            // its first successful refresh.
            model_catalog_verified: Arc::new(move || {
                catalog.as_ref().is_some_and(ModelCatalog::is_verified)
            }),
        }
    }
}

/// Builds the control-plane operational HTTP router. Product resources remain
/// available only through the canonical native gRPC API.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/livez", get(liveness))
        .route("/readyz", get(readiness))
        .route("/metrics", get(metrics))
        .layer(middleware::from_fn(observe_http))
        .with_state(HttpState::from(state))
}

async fn observe_http(request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
        .map_or_else(|| Uuid::now_v7().to_string(), str::to_owned);
    let method = request.method().to_string();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    let span = debug_span!("http.request", %request_id, %method, %route);
    async move {
        let started = Instant::now();
        let mut response = next.run(request).await;
        let status = response.status().as_u16();
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        debug!(status, latency_ms, "HTTP request completed");
        if let Ok(value) = HeaderValue::from_str(&request_id) {
            response.headers_mut().insert("x-request-id", value);
        }
        response
    }
    .instrument(span)
    .await
}

async fn liveness() -> impl IntoResponse {
    (StatusCode::OK, Json(HealthResponse::live()))
}
async fn metrics() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        application_metrics::render_clean_metrics(),
    )
}
async fn readiness(State(state): State<HttpState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse::ready(
            state.model_catalog_configured,
            (state.model_catalog_verified)(),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tower::ServiceExt;

    #[tokio::test]
    async fn operational_routes_allow_startup_without_model_configuration()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?;
        let service = router(state);
        for route in ["/livez", "/readyz", "/metrics"] {
            let response = service
                .clone()
                .oneshot(Request::builder().uri(route).body(Body::empty())?)
                .await?;
            assert_eq!(response.status(), StatusCode::OK);
            assert!(response.headers().contains_key("x-request-id"));
            if route == "/metrics" {
                assert_eq!(
                    response.headers()[header::CONTENT_TYPE],
                    "text/plain; version=0.0.4"
                );
            } else {
                let body = response.into_body().collect().await?.to_bytes();
                let health: serde_json::Value = serde_json::from_slice(&body)?;
                assert_eq!(health["provider_models"], "unconfigured");
                assert_eq!(health["component"], "control-plane");
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn operational_http_does_not_expose_product_resources()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?;
        let service = router(state);
        for route in ["/v1/repositories", "/v1/tasks", "/v1/runtime-profiles"] {
            let response = service
                .clone()
                .oneshot(Request::builder().uri(route).body(Body::empty())?)
                .await?;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        Ok(())
    }

    #[tokio::test]
    async fn probes_preserve_safe_request_ids_and_replace_unsafe_ones()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?;
        let service = router(state);
        for request_id in ["probe_1.2-safe", "contains spaces", ""] {
            let response = service
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/readyz")
                        .header("x-request-id", request_id)
                        .body(Body::empty())?,
                )
                .await?;
            let returned = response.headers()["x-request-id"].to_str()?;
            if request_id == "probe_1.2-safe" {
                assert_eq!(returned, request_id);
            } else {
                assert!(Uuid::parse_str(returned).is_ok());
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn readiness_reads_current_model_health_on_each_probe()
    -> Result<(), Box<dyn std::error::Error>> {
        let verified = Arc::new(AtomicBool::new(false));
        let observed = verified.clone();
        let state = HttpState {
            model_catalog_configured: true,
            model_catalog_verified: Arc::new(move || observed.load(Ordering::Acquire)),
        };
        for (available, expected) in [(false, "unavailable"), (true, "available")] {
            verified.store(available, Ordering::Release);
            let response = readiness(State(state.clone())).await.into_response();
            assert_eq!(response.status(), StatusCode::OK);
            let body = response.into_body().collect().await?.to_bytes();
            let health: serde_json::Value = serde_json::from_slice(&body)?;
            assert_eq!(health["provider_models"], expected);
        }
        Ok(())
    }

    #[tokio::test]
    async fn readiness_reports_the_configured_catalog_health()
    -> Result<(), Box<dyn std::error::Error>> {
        for (catalog, expected) in [
            (
                ModelCatalog::openai("unused-test-credential")?,
                "unavailable",
            ),
            (ModelCatalog::fixed(Vec::new()), "available"),
        ] {
            let directory = tempfile::tempdir()?;
            let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?
                .with_model_catalog(catalog);
            let response = router(state)
                .oneshot(Request::builder().uri("/readyz").body(Body::empty())?)
                .await?;
            let body = response.into_body().collect().await?.to_bytes();
            let health: serde_json::Value = serde_json::from_slice(&body)?;
            assert_eq!(health["provider_models"], expected);
        }
        Ok(())
    }
}
