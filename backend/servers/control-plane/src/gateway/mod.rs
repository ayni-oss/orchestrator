//! Local contributor gateway. It owns no workflow state or execution authority.
mod security;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use ayni_orchestrator_api::v1::{
    self, project_conversation_service_client::ProjectConversationServiceClient,
    project_service_client::ProjectServiceClient,
};
use futures_util::{Stream, StreamExt as _};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{convert::Infallible, io, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tonic::{
    Code, Status,
    transport::{Channel, Endpoint},
};

#[derive(Clone)]
pub struct Gateway {
    channel: Channel,
    authority: String,
    watches: Arc<Semaphore>,
    requests: Arc<Semaphore>,
}

impl Gateway {
    /// Construct a loopback-only gateway connected to a forwarded loopback API.
    ///
    /// # Errors
    /// Rejects non-loopback listeners/endpoints and malformed addresses.
    pub fn new(address: SocketAddr, endpoint: SocketAddr) -> Result<Self, io::Error> {
        if !address.ip().is_loopback() || !endpoint.ip().is_loopback() {
            return Err(io::Error::other("gateway and upstream must use loopback"));
        }
        let channel = Endpoint::from_shared(format!("http://{endpoint}"))
            .map_err(io::Error::other)?
            .connect_timeout(Duration::from_secs(3))
            .connect_lazy();
        Ok(Self {
            channel,
            authority: address.to_string(),
            watches: Arc::new(Semaphore::new(8)),
            requests: Arc::new(Semaphore::new(32)),
        })
    }

    fn connections(&self) -> v1::connection_service_client::ConnectionServiceClient<Channel> {
        v1::connection_service_client::ConnectionServiceClient::new(self.channel.clone())
    }

    fn projects(&self) -> ProjectServiceClient<Channel> {
        ProjectServiceClient::new(self.channel.clone()).max_decoding_message_size(3 * 1024 * 1024)
    }

    fn conversations(&self) -> ProjectConversationServiceClient<Channel> {
        ProjectConversationServiceClient::new(self.channel.clone())
            .max_decoding_message_size(3 * 1024 * 1024)
    }
}

/// Serve the explicitly allowlisted canonical API adapter without UI assets.
pub fn router(gateway: Gateway) -> Router {
    Router::new()
        .route("/api/{method}", post(rpc))
        .route("/watch", get(watch))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(
            gateway.clone(),
            security::guard,
        ))
        .with_state(gateway)
}

macro_rules! forward {
    ($body:expr, $client:expr, $method:ident, $request:ty) => {{
        let value: $request = decode($body)?;
        let mut request = tonic::Request::new(value);
        request.set_timeout(Duration::from_secs(10));
        let response = tokio::time::timeout(Duration::from_secs(12), $client.$method(request))
            .await
            .map_err(|_| {
                ApiError::from(Status::deadline_exceeded("workflow request timed out"))
            })??;
        encode(response.into_inner()).map(Json)
    }};
}

#[allow(
    clippy::too_many_lines,
    reason = "the conversation RPC allowlist is kept together for auditability"
)]
async fn rpc(
    State(gateway): State<Gateway>,
    Path(method): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let _permit = gateway
        .requests
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::from(Status::resource_exhausted("gateway request limit")))?;
    match method.as_str() {
        "SaveGithubConnection" => forward!(
            body,
            gateway.connections(),
            save_github_connection,
            v1::SaveGithubConnectionRequest
        ),
        "SetConnectionEnabled" => forward!(
            body,
            gateway.connections(),
            set_connection_enabled,
            v1::SetConnectionEnabledRequest
        ),
        "ValidateGithubConnection" => forward!(
            body,
            gateway.connections(),
            validate_github_connection,
            v1::ValidateGithubConnectionRequest
        ),
        "GetConnectionOperation" => forward!(
            body,
            gateway.connections(),
            get_connection_operation,
            v1::GetConnectionOperationRequest
        ),
        "ListConnections" => forward!(
            body,
            gateway.connections(),
            list_connections,
            v1::ListConnectionsRequest
        ),
        "ListConnectionOperations" => forward!(
            body,
            gateway.connections(),
            list_connection_operations,
            v1::ListConnectionOperationsRequest
        ),

        "CreatePlan" => forward!(
            body,
            gateway.conversations(),
            create_plan,
            v1::CreatePlanRequest
        ),
        "RevisePlan" => forward!(
            body,
            gateway.conversations(),
            revise_plan,
            v1::RevisePlanRequest
        ),
        "GetPlanVersion" => forward!(
            body,
            gateway.conversations(),
            get_plan_version,
            v1::GetPlanVersionRequest
        ),
        "ListPlans" => forward!(
            body,
            gateway.conversations(),
            list_plans,
            v1::ListPlansRequest
        ),
        "ListPlanVersions" => forward!(
            body,
            gateway.conversations(),
            list_plan_versions,
            v1::ListPlanVersionsRequest
        ),
        "ListConversationSessions" => forward!(
            body,
            gateway.conversations(),
            list_conversation_sessions,
            v1::ListConversationSessionsRequest
        ),
        "ReplaceConversationSession" => forward!(
            body,
            gateway.conversations(),
            replace_conversation_session,
            v1::ReplaceConversationSessionRequest
        ),
        "CreateConversation" => forward!(
            body,
            gateway.conversations(),
            create_conversation,
            v1::CreateConversationRequest
        ),
        "ListConversations" => forward!(
            body,
            gateway.conversations(),
            list_conversations,
            v1::ListConversationsRequest
        ),
        "ListProviderCalls" => forward!(
            body,
            gateway.conversations(),
            list_provider_calls,
            v1::ListProviderCallsRequest
        ),
        "AcknowledgeProviderUncertainty" => forward!(
            body,
            gateway.conversations(),
            acknowledge_provider_uncertainty,
            v1::AcknowledgeProviderUncertaintyRequest
        ),
        "CancelTurn" => forward!(
            body,
            gateway.conversations(),
            cancel_turn,
            v1::CancelTurnRequest
        ),
        "GetOutcomeAssessmentStatus" => forward!(
            body,
            gateway.conversations(),
            get_outcome_assessment_status,
            v1::GetOutcomeAssessmentStatusRequest
        ),
        "AuthorizeOutcomeAssessments" => forward!(
            body,
            gateway.conversations(),
            authorize_outcome_assessments,
            v1::AuthorizeOutcomeAssessmentsRequest
        ),
        "SendMessage" => forward!(
            body,
            gateway.conversations(),
            send_message,
            v1::SendMessageRequest
        ),
        "GetConversationSnapshot" => forward!(
            body,
            gateway.conversations(),
            get_conversation_snapshot,
            v1::GetConversationSnapshotRequest
        ),
        "ListExecutionProgress" => forward!(
            body,
            gateway.conversations(),
            list_execution_progress,
            v1::ListExecutionProgressRequest
        ),
        "ListExecutionTimeline" => forward!(
            body,
            gateway.conversations(),
            list_execution_timeline,
            v1::ListExecutionTimelineRequest
        ),
        "ListConversationHistory" => forward!(
            body,
            gateway.conversations(),
            list_conversation_history,
            v1::ListConversationHistoryRequest
        ),
        _ => task_rpc(&gateway, &method, body).await,
    }
}

// Task controls share the same request limits and canonical authority as plans.
async fn task_rpc(gateway: &Gateway, method: &str, body: Value) -> Result<Json<Value>, ApiError> {
    match method {
        "ConfirmTaskRecovery" => forward!(
            body,
            gateway.conversations(),
            confirm_task_recovery,
            v1::ConfirmTaskRecoveryRequest
        ),
        "RecordTaskRecovery" => forward!(
            body,
            gateway.conversations(),
            record_task_recovery,
            v1::RecordTaskRecoveryRequest
        ),
        "ListTaskRecovery" => forward!(
            body,
            gateway.conversations(),
            list_task_recovery,
            v1::ListTaskRecoveryRequest
        ),
        "ListTaskFeedback" => forward!(
            body,
            gateway.conversations(),
            list_task_feedback,
            v1::ListTaskFeedbackRequest
        ),
        "ReviewTaskPublication" => {
            let selected: v1::GetTaskAttemptRequest = decode(body)?;
            let preview = tokio::time::timeout(
                Duration::from_secs(12),
                crate::publication_review::review(
                    &mut gateway.conversations(),
                    selected,
                    &uuid::Uuid::now_v7().to_string(),
                ),
            )
            .await
            .map_err(|_| {
                ApiError::from(Status::deadline_exceeded("publication review timed out"))
            })??;
            encode(preview).map(Json)
        }
        "StartTaskPublication" => forward!(
            body,
            gateway.conversations(),
            start_task_publication,
            v1::StartTaskPublicationRequest
        ),
        "CancelTaskPublication" => forward!(
            body,
            gateway.conversations(),
            cancel_task_publication,
            v1::CancelTaskPublicationRequest
        ),
        "ApprovePlan" => forward!(
            body,
            gateway.conversations(),
            approve_plan,
            v1::ApprovePlanRequest
        ),
        "GetPlanApproval" => forward!(
            body,
            gateway.conversations(),
            get_plan_approval,
            v1::GetPlanApprovalRequest
        ),
        "ListTasks" => forward!(
            body,
            gateway.conversations(),
            list_tasks,
            v1::ListTasksRequest
        ),
        "GetTaskAttempt" => forward!(
            body,
            gateway.conversations(),
            get_task_attempt,
            v1::GetTaskAttemptRequest
        ),
        "CancelTaskExecution" => forward!(
            body,
            gateway.conversations(),
            cancel_task_execution,
            v1::CancelTaskExecutionRequest
        ),
        "GetTaskExecution" => forward!(
            body,
            gateway.conversations(),
            get_task_execution,
            v1::GetTaskExecutionRequest
        ),
        "GetTask" => forward!(body, gateway.conversations(), get_task, v1::GetTaskRequest),
        _ => project_rpc(gateway, method, body).await,
    }
}

type EventStream = Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>;

async fn project_rpc(
    gateway: &Gateway,
    method: &str,
    body: Value,
) -> Result<Json<Value>, ApiError> {
    match method {
        "CreateWorkspace" => forward!(
            body,
            gateway.projects(),
            create_workspace,
            v1::CreateWorkspaceRequest
        ),
        "CreateProject" => forward!(
            body,
            gateway.projects(),
            create_project,
            v1::CreateProjectRequest
        ),
        "ListWorkspaces" => forward!(
            body,
            gateway.projects(),
            list_workspaces,
            v1::ListWorkspacesRequest
        ),
        "ListProjects" => forward!(
            body,
            gateway.projects(),
            list_projects,
            v1::ListProjectsRequest
        ),
        "GetAgentConfiguration" => forward!(
            body,
            gateway.projects(),
            get_agent_configuration,
            v1::GetAgentConfigurationRequest
        ),
        "UpdateAgentConfiguration" => forward!(
            body,
            gateway.projects(),
            update_agent_configuration,
            v1::UpdateAgentConfigurationRequest
        ),
        "GetTurnContextInputs" => forward!(
            body,
            gateway.projects(),
            get_turn_context_inputs,
            v1::GetTurnContextInputsRequest
        ),
        "GetTurnInputs" => forward!(
            body,
            gateway.projects(),
            get_turn_inputs,
            v1::GetTurnInputsRequest
        ),
        _ => Err(ApiError::from(Status::unimplemented(
            "unknown gateway method",
        ))),
    }
}

async fn watch(
    State(gateway): State<Gateway>,
    headers: HeaderMap,
    Query(mut request): Query<v1::WatchConversationRequest>,
) -> Result<Response, ApiError> {
    let permit = gateway
        .watches
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::from(Status::resource_exhausted("gateway watch limit")))?;
    if let Some(value) = headers.get("last-event-id") {
        let value = value
            .to_str()
            .map_err(|_| ApiError::from(Status::invalid_argument("invalid cursor")))?;
        if value.len() > 4096 {
            return Err(ApiError::from(Status::invalid_argument("invalid cursor")));
        }
        request.after_cursor = Some(value.to_owned());
    }
    let mut client = gateway.conversations();
    let upstream =
        tokio::time::timeout(Duration::from_secs(10), client.watch_conversation(request)).await;
    let stream: EventStream = match upstream {
        Ok(Ok(response)) => Box::pin(async_stream::stream! {
            let _permit = permit;
            let mut upstream = response.into_inner();
            while let Some(frame) = upstream.next().await {
                match frame {
                    Ok(frame) => match frame_event(&frame) {
                        Ok(event) => yield Ok(event),
                        Err(error) => { yield Ok(fault(&error)); break; }
                    },
                    Err(error) => { yield Ok(fault(&error)); break; }
                }
            }
        }),
        result => {
            let error = match result {
                Ok(Err(error)) => error,
                _ => Status::deadline_exceeded("watch connection timed out"),
            };
            Box::pin(futures_util::stream::once(async move {
                let _permit = permit;
                Ok(fault(&error))
            }))
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

fn frame_event(frame: &v1::WatchConversationResponse) -> Result<Event, Status> {
    let mut event = Event::default()
        .event("frame")
        .json_data(frame)
        .map_err(|_| Status::internal("frame encoding failed"))?;
    let cursor = match &frame.frame {
        Some(v1::watch_conversation_response::Frame::Snapshot(value)) => Some(&value.resume_cursor),
        Some(v1::watch_conversation_response::Frame::Update(value)) => Some(&value.cursor),
        _ => None,
    };
    if let Some(cursor) = cursor {
        event = event.id(cursor);
    }
    Ok(event)
}

fn fault(status: &Status) -> Event {
    Event::default()
        .event("fault")
        .data(error_body(status).to_string())
}

fn decode<T: DeserializeOwned>(body: Value) -> Result<T, ApiError> {
    serde_json::from_value(body)
        .map_err(|_| ApiError::from(Status::invalid_argument("invalid request JSON")))
}

fn encode(value: impl Serialize) -> Result<Value, ApiError> {
    serde_json::to_value(value)
        .map_err(|_| ApiError::from(Status::internal("response encoding failed")))
}

fn error_body(status: &Status) -> Value {
    use prost::Message as _;
    let reason = RichStatus::decode(status.details())
        .ok()
        .and_then(|rich| {
            rich.details.into_iter().find(|detail| {
                detail.type_url == "type.googleapis.com/ayni.orchestrator.v1.ErrorDetail"
            })
        })
        .and_then(|detail| v1::ErrorDetail::decode(detail.value.as_slice()).ok())
        .and_then(|detail| v1::ErrorReason::try_from(detail.reason).ok())
        .map(|reason| reason.as_str_name());
    json!({"code": format!("{:?}", status.code()), "reason": reason, "message": "Workflow request could not complete", "requestId": status.metadata().get("x-request-id").and_then(|value| value.to_str().ok())})
}

#[derive(Clone, PartialEq, prost::Message)]
struct RichStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<prost_types::Any>,
}

pub struct ApiError(Status);
impl From<Status> for ApiError {
    fn from(value: Status) -> Self {
        Self(value)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code = match self.0.code() {
            Code::InvalidArgument => StatusCode::BAD_REQUEST,
            Code::NotFound | Code::Unimplemented => StatusCode::NOT_FOUND,
            Code::AlreadyExists | Code::Aborted | Code::FailedPrecondition => StatusCode::CONFLICT,
            Code::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
            Code::Unavailable | Code::DeadlineExceeded => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (code, Json(error_body(&self.0))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, header},
    };
    use prost::Message as _;
    use tower::ServiceExt as _;

    #[tokio::test]
    async fn local_boundary_rejects_rebinding_cross_origin_and_simple_posts()
    -> Result<(), Box<dyn std::error::Error>> {
        let gateway = Gateway::new("127.0.0.1:3000".parse()?, "127.0.0.1:1".parse()?)?;
        let app = router(gateway);
        for (host, origin, fetch_site, method) in [
            (
                "attacker.example:3000",
                "http://127.0.0.1:3000",
                "same-origin",
                "GET",
            ),
            (
                "127.0.0.1:3000",
                "https://attacker.example",
                "cross-site",
                "GET",
            ),
            (
                "127.0.0.1:3000",
                "http://127.0.0.1:3000",
                "cross-site",
                "GET",
            ),
            (
                "127.0.0.1:3000",
                "http://127.0.0.1:3000",
                "same-origin",
                "POST",
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/")
                        .method(method)
                        .header("host", host)
                        .header("origin", origin)
                        .header("sec-fetch-site", fetch_site)
                        .body(Body::empty())?,
                )
                .await?;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("host", "127.0.0.1:3000")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(
            response.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()?
                .contains("frame-ancestors 'none'")
        );
        for path in [
            "/workspace/",
            "/workspace/index.html",
            "/app.mjs",
            "/style.css",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("host", "127.0.0.1:3000")
                        .body(Body::empty())?,
                )
                .await?;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        assert!(Gateway::new("0.0.0.0:3000".parse()?, "127.0.0.1:1".parse()?).is_err());
        assert!(Gateway::new("127.0.0.1:3000".parse()?, "192.0.2.1:50051".parse()?).is_err());
        Ok(())
    }

    #[test]
    fn protojson_keeps_64_bit_revisions_and_typed_expiry() -> Result<(), Box<dyn std::error::Error>>
    {
        let value = v1::ConversationSnapshot {
            conversation_revision: 9_007_199_254_740_993,
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(value)?["conversationRevision"],
            "9007199254740993"
        );
        let detail = v1::ErrorDetail {
            reason: v1::ErrorReason::WatchCursorExpired as i32,
            ..Default::default()
        };
        let status = Status::with_details(
            Code::FailedPrecondition,
            "private content",
            RichStatus {
                code: 9,
                message: String::new(),
                details: vec![prost_types::Any {
                    type_url: "type.googleapis.com/ayni.orchestrator.v1.ErrorDetail".into(),
                    value: detail.encode_to_vec(),
                }],
            }
            .encode_to_vec()
            .into(),
        );
        assert_eq!(
            error_body(&status)["reason"],
            "ERROR_REASON_WATCH_CURSOR_EXPIRED"
        );
        assert!(!error_body(&status).to_string().contains("private content"));
        Ok(())
    }
}
