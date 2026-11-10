//! Runtime workers hold one stage capability; only supervisors adopt and retire Pods.
pub(crate) mod progress;
use axum::{
    Json, Router,
    extract::Request,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use ayni_orchestrator_events::runtime_preparation::{
    Binding, MAX_WORKER_REQUEST_BYTES, STAGES, StageClaim, StageReport, SupervisorRequest,
    WorkerRequest, WorkerResponse, worker_token,
};
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, store::runtime_preparations::Credential,
};
use sha2::{Digest as _, Sha256};
use std::{io, sync::Arc};
use subtle::ConstantTimeEq as _;

#[derive(Clone)]
struct Authority {
    store: WorkflowStore,
    token: Arc<str>,
}

/// Install only when an administrator explicitly mounts runtime supervisor authority.
/// # Errors
/// Reject malformed or unavailable configured authority rather than opening endpoints.
pub fn router(store: WorkflowStore) -> Result<Router, io::Error> {
    let path = match std::env::var("ORCHESTRATOR_RUNTIME_AUTHORITY_TOKEN_PATH") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return Ok(Router::new()),
        Err(_) => return Err(io::Error::other("runtime authority configuration invalid")),
    };
    let token = std::fs::read_to_string(path)
        .map_err(|_| io::Error::other("runtime authority unavailable"))?;
    let token = token.trim();
    if !(32..=256).contains(&token.len()) {
        return Err(io::Error::other("runtime authority invalid"));
    }
    Ok(Router::new()
        .route("/v1/runtime-supervisor", post(supervisor))
        .route(
            "/v1/runtime-supervisor/diagnostics",
            post(retain_diagnostic).get(replay_diagnostics),
        )
        .route(
            "/v1/runtime-supervisor/diagnostics/stream",
            axum::routing::get(tail_diagnostics),
        )
        .route(
            "/v1/runtime-supervisor/certification",
            post(supervisor_certification),
        )
        .route(
            "/v1/runtime-supervisor/credential",
            post(supervisor_pull_credential),
        )
        .route(
            "/v1/runtime-worker/{operation}/{stage}/{generation}",
            post(worker),
        )
        .route(
            "/v1/runtime-worker/{operation}/{stage}/{generation}/credential",
            post(credential),
        )
        .layer(DefaultBodyLimit::max(MAX_WORKER_REQUEST_BYTES))
        .layer(middleware::from_fn(authority_status))
        .with_state(Authority {
            store,
            token: Arc::from(token),
        }))
}

async fn authority_status(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    if !response.status().is_success() {
        tracing::warn!(
            status = response.status().as_u16(),
            "Runtime authority request rejected"
        );
    }
    response
}

fn authorized(headers: &HeaderMap, expected: &str) -> Result<(), StatusCode> {
    let actual = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if bool::from(Sha256::digest(actual.as_bytes()).ct_eq(&Sha256::digest(expected.as_bytes()))) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn retain_diagnostic(
    State(authority): State<Authority>,
    headers: HeaderMap,
    Json(diagnostic): Json<ayni_orchestrator_events::execution_diagnostics::Diagnostic>,
) -> Result<Json<()>, StatusCode> {
    authorized(&headers, &authority.token)?;
    crate::execution_diagnostics::retain(&authority.store, diagnostic, true).await
}

async fn replay_diagnostics(
    State(authority): State<Authority>,
    headers: HeaderMap,
    Query(query): Query<crate::execution_diagnostics::Replay>,
) -> Result<Json<Vec<ayni_orchestrator_events::execution_diagnostics::Entry>>, StatusCode> {
    authorized(&headers, &authority.token)?;
    crate::execution_diagnostics::replay(&authority.store, query, true).await
}

async fn tail_diagnostics(
    State(authority): State<Authority>,
    headers: HeaderMap,
    Query(query): Query<crate::execution_diagnostics::Replay>,
) -> Result<Response, StatusCode> {
    authorized(&headers, &authority.token)?;
    Ok(crate::execution_diagnostics::tail(authority.store, query, true, &headers)?.into_response())
}

/// Only trusted supervision may obtain node-pull authority before Pod creation.
/// It is never attached to the candidate's process or command input.
async fn supervisor_pull_credential(
    State(authority): State<Authority>,
    headers: HeaderMap,
    Json(binding): Json<Binding>,
) -> Result<Response, StatusCode> {
    authorized(&headers, &authority.token)?;
    let credential = authority
        .store
        .runtime_supervisor_pull_credential(&binding)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    let Credential::Registry { username, password } = credential else {
        return Err(StatusCode::FORBIDDEN);
    };
    Ok(private_response(
        serde_json::json!({"kind":"registry","username":username,"password":password}),
    ))
}

/// Certification is accepted only after the supervisor has observed termination
/// and rechecked the exact immutable signing Secret UID.
async fn supervisor_certification(
    State(authority): State<Authority>,
    headers: HeaderMap,
    Json(report): Json<StageReport>,
) -> Result<Json<WorkerResponse>, StatusCode> {
    authorized(&headers, &authority.token)?;
    if report.binding.stage != "certify" {
        return Err(StatusCode::FORBIDDEN);
    }
    authority
        .store
        .report_runtime_stage(&report)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    log_stage_report(&report);
    Ok(Json(WorkerResponse {
        claim: None,
        accepted: true,
    }))
}

fn log_stage_report(report: &StageReport) {
    // Called only after the authority validates the binding and closed outcome/reason.
    let binding = &report.binding;
    tracing::info!(operation_id = %binding.operation_id, stage = %binding.stage,
        generation = binding.generation, pod_uid = %binding.pod_uid,
        outcome = %report.outcome, reason = %report.safe_reason, "Runtime stage report accepted");
}

fn supervisor_store_failure(
    action: &'static str,
    operation_id: &str,
    error: &StoreError,
) -> StatusCode {
    let sql_state = match error {
        StoreError::Database(source) => source.code().map_or("", |code| code.code()),
        _ => "",
    };
    tracing::warn!(action, operation_id, reason=%error, sql_state, "Runtime supervisor authority request failed");
    StatusCode::CONFLICT
}

async fn supervisor(
    State(authority): State<Authority>,
    headers: HeaderMap,
    Json(request): Json<SupervisorRequest>,
) -> Result<Json<Option<StageClaim>>, StatusCode> {
    authorized(&headers, &authority.token)?;
    let result = match request {
        SupervisorRequest::Poll => {
            let pending = authority
                .store
                .pending_runtime_preparations(1)
                .await
                .map_err(|error| supervisor_store_failure("poll", "", &error))?;
            match pending.first() {
                Some(operation) => Some(
                    authority
                        .store
                        .claim_runtime_stage(&operation.id)
                        .await
                        .map_err(|error| {
                            supervisor_store_failure("claim", &operation.id, &error)
                        })?,
                ),
                None => None,
            }
        }
        SupervisorRequest::Bind { binding } => {
            authority
                .store
                .bind_runtime_stage(&binding)
                .await
                .map_err(|_| StatusCode::CONFLICT)?;
            None
        }
        SupervisorRequest::FailedBeforeStart {
            binding,
            safe_reason,
        } => {
            authority
                .store
                .fail_unbound_runtime_stage(&binding, &safe_reason)
                .await
                .map_err(|_| StatusCode::CONFLICT)?;
            None
        }
        SupervisorRequest::Stopped { binding } => {
            authority
                .store
                .stop_runtime_stage(&binding)
                .await
                .map_err(|_| StatusCode::CONFLICT)?;
            None
        }
    };
    Ok(Json(result))
}

fn binding(
    authority: &Authority,
    route: (String, String, u64),
    headers: &HeaderMap,
) -> Result<Binding, StatusCode> {
    let (operation_id, stage, generation) = route;
    uuid::Uuid::parse_str(&operation_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    if generation == 0 || !STAGES.contains(&stage.as_str()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let pod_uid = header_text(headers, "x-runtime-pod-uid")?;
    uuid::Uuid::parse_str(&pod_uid).map_err(|_| StatusCode::BAD_REQUEST)?;
    let command_digest = header_text(headers, "x-runtime-command-digest")?;
    if !canonical_digest(&command_digest) {
        return Err(StatusCode::BAD_REQUEST);
    }
    authorized(
        headers,
        &worker_token(
            authority.token.as_bytes(),
            &operation_id,
            &stage,
            generation,
            &command_digest,
        ),
    )?;
    Ok(Binding {
        operation_id,
        stage,
        generation,
        pod_uid,
        command_digest,
    })
}

fn header_text(headers: &HeaderMap, name: &'static str) -> Result<String, StatusCode> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 128)
        .map(str::to_owned)
        .ok_or(StatusCode::UNAUTHORIZED)
}

fn canonical_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    })
}

async fn worker(
    State(authority): State<Authority>,
    Path(route): Path<(String, String, u64)>,
    headers: HeaderMap,
    Json(request): Json<WorkerRequest>,
) -> Result<Json<WorkerResponse>, StatusCode> {
    let binding = binding(&authority, route, &headers)?;
    let mut response = WorkerResponse::default();
    match request {
        WorkerRequest::Input => {
            response.claim = Some(
                authority
                    .store
                    .runtime_stage_input(&binding)
                    .await
                    .map_err(|_| StatusCode::CONFLICT)?,
            );
        }
        WorkerRequest::BuildProgress { milestone } => {
            if binding.stage != "build" {
                return Err(StatusCode::FORBIDDEN);
            }
            authority
                .store
                .record_runtime_build_checkpoint(&binding, milestone)
                .await
                .map_err(|_| StatusCode::CONFLICT)?;
            progress::record(&binding, milestone);
            response.accepted = true;
        }
        WorkerRequest::BuildOutput { output } => {
            if binding.stage != "build" || !output.valid() {
                return Err(StatusCode::BAD_REQUEST);
            }
            authority
                .store
                .record_runtime_build_output(&binding, output)
                .await
                .map_err(|_| StatusCode::CONFLICT)?;
            response.accepted = true;
        }
        WorkerRequest::PreparePublication {
            expected_manifest_digest,
        } => {
            authority
                .store
                .worker_prepare_publication(&binding, &expected_manifest_digest)
                .await
                .map_err(|_| StatusCode::CONFLICT)?;
            response.accepted = true;
        }
        WorkerRequest::Report { report } => {
            if binding.stage == "certify" {
                return Err(StatusCode::FORBIDDEN);
            }
            if report.binding != binding {
                return Err(StatusCode::CONFLICT);
            }
            let elapsed_ms = progress::snapshot(&binding.operation_id, binding.generation)
                .map_or_else(Default::default, |value| value.elapsed_ms);
            authority
                .store
                .report_runtime_stage_with_timings(&report, elapsed_ms)
                .await
                .map_err(|_| StatusCode::CONFLICT)?;
            log_stage_report(&report);
            response.accepted = true;
        }
    }
    Ok(Json(response))
}

async fn credential(
    State(authority): State<Authority>,
    Path(route): Path<(String, String, u64)>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let binding = binding(&authority, route, &headers)?;
    let credential = authority
        .store
        .runtime_stage_credential(&binding)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    let value = match credential {
        Credential::Source {
            credential,
            installation_id,
            repository_id,
        } => {
            let token = tokio::time::timeout(
                std::time::Duration::from_secs(20),
                crate::workflow::installation_read_token(
                    &credential,
                    installation_id,
                    repository_id,
                ),
            )
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .map_err(provider_status)?;
            serde_json::json!({"kind":"github_installation","token":token})
        }
        Credential::Registry { username, password } => {
            serde_json::json!({"kind":"registry","username":username,"password":password})
        }
        Credential::None => return Err(StatusCode::FORBIDDEN),
    };
    // A slow provider response must not deliver authority after cancellation/rotation.
    authority
        .store
        .runtime_stage_input(&binding)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    Ok(private_response(value))
}

fn private_response(value: serde_json::Value) -> Response {
    let mut response = Json(value).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

fn provider_status(reason: &str) -> StatusCode {
    match reason {
        "github_access_denied" | "github_repository_unavailable" => StatusCode::FORBIDDEN,
        "github_authentication_failed" | "github_key_invalid" => StatusCode::UNAUTHORIZED,
        "github_rate_limited" => StatusCode::TOO_MANY_REQUESTS,
        "github_unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::BAD_GATEWAY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn certification_requires_supervisor_and_cannot_use_worker_reporting()
    -> Result<(), Box<dyn std::error::Error>> {
        let secret = "synthetic-authority-01234567890123456789";
        let authority = Authority {
            store: WorkflowStore::new("unused"),
            token: Arc::from(secret),
        };
        let binding = Binding {
            operation_id: uuid::Uuid::new_v4().to_string(),
            stage: "certify".into(),
            generation: 1,
            pod_uid: uuid::Uuid::new_v4().to_string(),
            command_digest: format!("sha256:{}", "1".repeat(64)),
        };
        let report = StageReport {
            binding: binding.clone(),
            outcome: "success".into(),
            safe_reason: String::new(),
            evidence: ayni_orchestrator_events::runtime_preparation::Evidence::default(),
        };
        let token = worker_token(
            secret.as_bytes(),
            &binding.operation_id,
            &binding.stage,
            binding.generation,
            &binding.command_digest,
        );
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, format!("Bearer {token}").parse()?);
        headers.insert("x-runtime-pod-uid", binding.pod_uid.parse()?);
        headers.insert("x-runtime-command-digest", binding.command_digest.parse()?);
        assert_eq!(
            supervisor_certification(
                State(authority.clone()),
                headers.clone(),
                Json(report.clone())
            )
            .await
            .err(),
            Some(StatusCode::UNAUTHORIZED)
        );
        assert_eq!(
            worker(
                State(authority.clone()),
                Path((binding.operation_id, binding.stage, binding.generation)),
                headers.clone(),
                Json(WorkerRequest::Report {
                    report: Box::new(report.clone())
                })
            )
            .await
            .err(),
            Some(StatusCode::FORBIDDEN)
        );
        headers.insert(header::AUTHORIZATION, format!("Bearer {secret}").parse()?);
        let mut wrong_stage = report;
        wrong_stage.binding.stage = "build".into();
        assert_eq!(
            supervisor_certification(State(authority), headers, Json(wrong_stage))
                .await
                .err(),
            Some(StatusCode::FORBIDDEN)
        );
        Ok(())
    }
    #[test]
    fn worker_cannot_change_scope_or_use_supervisor_authority() {
        let secret = "synthetic-authority-01234567890123456789";
        let operation = uuid::Uuid::new_v4().to_string();
        let digest = format!("sha256:{}", "1".repeat(64));
        let token = worker_token(secret.as_bytes(), &operation, "build", 1, &digest);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .unwrap_or_else(|error| panic!("fixture header: {error}")),
        );
        assert!(authorized(&headers, secret).is_err());
        assert!(
            authorized(
                &headers,
                &worker_token(secret.as_bytes(), &operation, "publish", 1, &digest)
            )
            .is_err()
        );
        assert!(
            authorized(
                &headers,
                &worker_token(secret.as_bytes(), &operation, "build", 2, &digest)
            )
            .is_err()
        );
        assert!(authorized(&headers, &token).is_ok());
    }
}
