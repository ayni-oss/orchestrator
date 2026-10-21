use ayni_orchestrator_api::Timestamp;
use ayni_orchestrator_api::{FILE_DESCRIPTOR_SET, v1, validation};
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore,
    model::{
        self, DispatchState, ExecutionState, HistoryKind, MessageRole, PresentationState,
        ReceiptKind, SchedulingState,
    },
    store::observation::{ConversationUpdate, WatchEvent, WatchOptions},
};
use prost::Message as _;
use prost_types::Any;
use std::{io, net::SocketAddr, pin::Pin};
use tokio::sync::watch;
use tokio_stream::{Stream, StreamExt as _};
use tonic::{Code, Request, Response, Status, transport::Server};
use uuid::Uuid;

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 3 * 1024 * 1024;
const ERROR_DETAIL_TYPE_URL: &str = "type.googleapis.com/ayni.orchestrator.v1.ErrorDetail";
type ConversationStream =
    Pin<Box<dyn Stream<Item = Result<v1::WatchConversationResponse, Status>> + Send + 'static>>;

#[derive(Clone)]
pub struct WorkflowGrpc {
    store: WorkflowStore,
}

impl WorkflowGrpc {
    #[must_use]
    pub const fn new(store: WorkflowStore) -> Self {
        Self { store }
    }
}

/// Serve the trusted-loopback M1 workflow API.
///
/// # Errors
/// Returns an I/O error when the listener or reflection service fails.
pub async fn serve(
    address: SocketAddr,
    store: WorkflowStore,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), io::Error> {
    if !address.ip().is_loopback() {
        return Err(io::Error::other(
            "trusted workflow gRPC listener must use loopback",
        ));
    }
    let api = WorkflowGrpc::new(store);
    let (health_reporter, health) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<v1::project_service_server::ProjectServiceServer<WorkflowGrpc>>()
        .await;
    health_reporter
        .set_serving::<
            v1::project_conversation_service_server::ProjectConversationServiceServer<WorkflowGrpc>,
        >()
        .await;
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
        .build_v1()
        .map_err(io::Error::other)?;
    Server::builder()
        .add_service(health)
        .add_service(reflection)
        .add_service(
            v1::project_service_server::ProjectServiceServer::new(api.clone())
                .max_decoding_message_size(MAX_REQUEST_BYTES)
                .max_encoding_message_size(MAX_RESPONSE_BYTES),
        )
        .add_service(
            v1::project_conversation_service_server::ProjectConversationServiceServer::new(api)
                .max_decoding_message_size(MAX_REQUEST_BYTES)
                .max_encoding_message_size(MAX_RESPONSE_BYTES),
        )
        .serve_with_shutdown(address, async move {
            if !*shutdown.borrow() {
                let _ = shutdown.changed().await;
            }
        })
        .await
        .map_err(io::Error::other)
}

#[tonic::async_trait]
impl v1::project_service_server::ProjectService for WorkflowGrpc {
    async fn get_turn_context_inputs(
        &self,
        request: Request<v1::GetTurnContextInputsRequest>,
    ) -> Result<Response<v1::GetTurnContextInputsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let page = self
            .store
            .get_turn_context_inputs(
                &value.workspace_id,
                &value.project_id,
                &value.conversation_id,
                &value.turn_id,
                value.after_ordinal,
                value.limit,
            )
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::GetTurnContextInputsResponse {
            manifest_id: page.manifest.id,
            digest: page.manifest.digest,
            record_count: page.manifest.record_count,
            records: page
                .records
                .into_iter()
                .map(|r| v1::ContextInputRecord {
                    message_id: r.message_id,
                    sequence: r.sequence,
                    role: r.role,
                    content: r.content,
                })
                .collect(),
            next_after_ordinal: page.next_after_ordinal,
        }))
    }

    async fn get_agent_configuration(
        &self,
        request: Request<v1::GetAgentConfigurationRequest>,
    ) -> Result<Response<v1::GetAgentConfigurationResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let result = self
            .store
            .get_agent_configuration(&value.workspace_id, &value.revision_id)
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::GetAgentConfigurationResponse {
            configuration: Some(agent_configuration(result)),
        }))
    }
    async fn update_agent_configuration(
        &self,
        request: Request<v1::UpdateAgentConfigurationRequest>,
    ) -> Result<Response<v1::UpdateAgentConfigurationResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let proposed = value.configuration.ok_or_else(|| {
            context.status(
                Code::InvalidArgument,
                v1::ErrorReason::InvalidInput,
                "configuration is required",
            )
        })?;
        let proposed = model::AgentConfiguration {
            workspace_id: proposed.workspace_id,
            revision_id: proposed.revision_id,
            instructions: proposed.instructions,
            provider: proposed.provider,
            model: proposed.model,
            max_input_tokens: proposed.max_input_tokens,
            max_output_tokens: proposed.max_output_tokens,
            max_total_tokens: proposed.max_total_tokens,
            idle_timeout_seconds: proposed.idle_timeout_seconds,
        };
        let result = self
            .store
            .update_agent_configuration(
                &value.idempotency_key,
                &value.expected_revision_id,
                &proposed,
            )
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::UpdateAgentConfigurationResponse {
            configuration: Some(agent_configuration(result)),
        }))
    }
    async fn get_turn_inputs(
        &self,
        request: Request<v1::GetTurnInputsRequest>,
    ) -> Result<Response<v1::GetTurnInputsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let result = self
            .store
            .get_turn_inputs(
                &value.workspace_id,
                &value.project_id,
                &value.conversation_id,
                &value.turn_id,
            )
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::GetTurnInputsResponse {
            turn_id: result.turn_id,
            configuration: Some(agent_configuration(result.configuration)),
        }))
    }

    async fn list_workspaces(
        &self,
        request: Request<v1::ListWorkspacesRequest>,
    ) -> Result<Response<v1::ListWorkspacesResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let page = self
            .store
            .list_workspaces(&value.after_id, value.limit)
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::ListWorkspacesResponse {
            workspaces: page.resources.into_iter().map(workspace).collect(),
            next_after_id: page.next_after_id,
        }))
    }

    async fn list_projects(
        &self,
        request: Request<v1::ListProjectsRequest>,
    ) -> Result<Response<v1::ListProjectsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let page = self
            .store
            .list_projects(&value.workspace_id, &value.after_id, value.limit)
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::ListProjectsResponse {
            projects: page.resources.into_iter().map(project).collect(),
            next_after_id: page.next_after_id,
        }))
    }

    async fn create_workspace(
        &self,
        request: Request<v1::CreateWorkspaceRequest>,
    ) -> Result<Response<v1::CreateWorkspaceResponse>, Status> {
        let context = RequestContext::new(&request);
        validation::create_workspace(request.get_ref())
            .map_err(|error| context.validation(&error))?;
        let request = request.into_inner();
        let created = self
            .store
            .create_workspace(&request.idempotency_key, &request.display_name)
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::CreateWorkspaceResponse {
            workspace: Some(workspace(created.resource)),
            receipt: Some(receipt(created.receipt)),
        }))
    }

    async fn create_project(
        &self,
        request: Request<v1::CreateProjectRequest>,
    ) -> Result<Response<v1::CreateProjectResponse>, Status> {
        let context = RequestContext::new(&request);
        validation::create_project(request.get_ref())
            .map_err(|error| context.validation(&error))?;
        let request = request.into_inner();
        let created = self
            .store
            .create_project(
                &request.idempotency_key,
                &request.workspace_id,
                &request.display_name,
                &request.goal,
            )
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::CreateProjectResponse {
            project: Some(project(created.resource)),
            receipt: Some(receipt(created.receipt)),
        }))
    }
}

#[tonic::async_trait]
impl v1::project_conversation_service_server::ProjectConversationService for WorkflowGrpc {
    async fn list_conversation_sessions(
        &self,
        request: Request<v1::ListConversationSessionsRequest>,
    ) -> Result<Response<v1::ListConversationSessionsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let sessions = self
            .store
            .conversation_sessions(
                &value.workspace_id,
                &value.project_id,
                &value.conversation_id,
            )
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::ListConversationSessionsResponse {
            sessions: sessions.into_iter().map(session).collect(),
        }))
    }

    async fn replace_conversation_session(
        &self,
        request: Request<v1::ReplaceConversationSessionRequest>,
    ) -> Result<Response<v1::ReplaceConversationSessionResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let replaced = self
            .store
            .replace_idle_session(
                &value.workspace_id,
                &value.project_id,
                &value.conversation_id,
                value.expected_generation,
                &value.idempotency_key,
            )
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::ReplaceConversationSessionResponse {
            session: Some(session(replaced)),
        }))
    }

    async fn list_conversations(
        &self,
        request: Request<v1::ListConversationsRequest>,
    ) -> Result<Response<v1::ListConversationsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let page = self
            .store
            .list_conversations(
                &value.workspace_id,
                &value.project_id,
                &value.after_id,
                value.limit,
            )
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::ListConversationsResponse {
            conversations: page.resources.into_iter().map(conversation).collect(),
            next_after_id: page.next_after_id,
        }))
    }

    async fn create_conversation(
        &self,
        request: Request<v1::CreateConversationRequest>,
    ) -> Result<Response<v1::CreateConversationResponse>, Status> {
        let context = RequestContext::new(&request);
        validation::create_conversation(request.get_ref())
            .map_err(|error| context.validation(&error))?;
        let request = request.into_inner();
        let created = self
            .store
            .create_conversation(
                &request.idempotency_key,
                &request.workspace_id,
                &request.project_id,
                &request.title,
            )
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::CreateConversationResponse {
            conversation: Some(conversation(created.resource)),
            receipt: Some(receipt(created.receipt)),
        }))
    }

    async fn send_message(
        &self,
        request: Request<v1::SendMessageRequest>,
    ) -> Result<Response<v1::SendMessageResponse>, Status> {
        let context = RequestContext::new(&request);
        validation::send_message(request.get_ref()).map_err(|error| context.validation(&error))?;
        let request = request.into_inner();
        let accepted = self
            .store
            .send_message(
                &request.idempotency_key,
                &request.workspace_id,
                &request.project_id,
                &request.conversation_id,
                &request.content,
            )
            .await
            .map_err(|error| context.store(&error))?;
        let handoff = std::time::Instant::now();
        let response = context.response(v1::SendMessageResponse {
            receipt: Some(receipt(accepted)),
        });
        ayni_orchestrator_workflow_postgres::metrics::observe(
            "workflow_receipt_handoff_seconds",
            handoff.elapsed(),
        );
        Ok(response)
    }

    async fn get_conversation_snapshot(
        &self,
        request: Request<v1::GetConversationSnapshotRequest>,
    ) -> Result<Response<v1::GetConversationSnapshotResponse>, Status> {
        let context = RequestContext::new(&request);
        let request = request.into_inner();
        validation::conversation_scope(
            &request.workspace_id,
            &request.project_id,
            &request.conversation_id,
        )
        .map_err(|error| context.validation(&error))?;
        let durable_snapshot = self
            .store
            .get_snapshot(
                &request.workspace_id,
                &request.project_id,
                &request.conversation_id,
            )
            .await
            .map_err(|error| context.store(&error))?;
        let response = v1::GetConversationSnapshotResponse {
            snapshot: Some(snapshot(durable_snapshot)),
        };
        if response.encoded_len() > MAX_RESPONSE_BYTES {
            return Err(context.status(
                Code::ResourceExhausted,
                v1::ErrorReason::BoundExceeded,
                "workflow snapshot exceeds response bound",
            ));
        }
        Ok(context.response(response))
    }

    async fn list_conversation_history(
        &self,
        request: Request<v1::ListConversationHistoryRequest>,
    ) -> Result<Response<v1::ListConversationHistoryResponse>, Status> {
        let context = RequestContext::new(&request);
        validation::history(request.get_ref()).map_err(|error| context.validation(&error))?;
        let request = request.into_inner();
        let page = self
            .store
            .list_history(
                &request.workspace_id,
                &request.project_id,
                &request.conversation_id,
                &request.snapshot_id,
                request.before_sequence,
                request.limit,
            )
            .await
            .map_err(|error| context.store(&error))?;
        let response = v1::ListConversationHistoryResponse {
            records: page.records.into_iter().map(history_record).collect(),
            snapshot_id: page.snapshot_id,
            next_before_sequence: page.next_before_sequence,
            has_more: page.has_more,
        };
        if response.encoded_len() > MAX_RESPONSE_BYTES {
            return Err(context.status(
                Code::ResourceExhausted,
                v1::ErrorReason::BoundExceeded,
                "workflow history exceeds response bound",
            ));
        }
        Ok(context.response(response))
    }

    type WatchConversationStream = ConversationStream;

    async fn watch_conversation(
        &self,
        request: Request<v1::WatchConversationRequest>,
    ) -> Result<Response<Self::WatchConversationStream>, Status> {
        let context = RequestContext::new(&request);
        let request = request.into_inner();
        validation::conversation_scope(
            &request.workspace_id,
            &request.project_id,
            &request.conversation_id,
        )
        .map_err(|error| context.validation(&error))?;
        let stream = self
            .store
            .watch(
                &request.workspace_id,
                &request.project_id,
                &request.conversation_id,
                request.after_cursor.as_deref(),
                WatchOptions::default(),
            )
            .await
            .map_err(|error| context.store(&error))?;
        let response_context = context.clone();
        Ok(context.response(Box::pin(stream.map(move |event| {
            let frame = match event.map_err(|error| response_context.store(&error))? {
                WatchEvent::Snapshot(value) => {
                    let value = snapshot(*value);
                    if value.encoded_len() > MAX_RESPONSE_BYTES - 16 {
                        return Err(response_context.status(
                            Code::ResourceExhausted,
                            v1::ErrorReason::BoundExceeded,
                            "workflow snapshot exceeds response bound",
                        ));
                    }
                    v1::watch_conversation_response::Frame::Snapshot(value)
                }
                WatchEvent::Update(value) => v1::watch_conversation_response::Frame::Update(
                    update(value).map_err(|error| response_context.store(&error))?,
                ),
                WatchEvent::Heartbeat => v1::watch_conversation_response::Frame::Heartbeat(
                    v1::ConversationWatchHeartbeat {},
                ),
            };
            Ok(v1::WatchConversationResponse { frame: Some(frame) })
        })) as ConversationStream))
    }
}

#[derive(Clone)]
struct RequestContext {
    request_id: String,
}

impl RequestContext {
    fn new<T>(request: &Request<T>) -> Self {
        let request_id = request
            .metadata()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .filter(|value| valid_request_id(value))
            .map_or_else(|| Uuid::now_v7().to_string(), str::to_owned);
        Self { request_id }
    }

    fn response<T>(&self, value: T) -> Response<T> {
        let mut response = Response::new(value);
        if let Ok(value) = self.request_id.parse() {
            response.metadata_mut().insert("x-request-id", value);
        }
        response
    }

    fn validation(&self, error: &validation::ValidationError) -> Status {
        self.status(
            Code::InvalidArgument,
            error.reason,
            "workflow request validation failed",
        )
    }

    fn store(&self, error: &StoreError) -> Status {
        match error {
            StoreError::InvalidInput(_) => self.status(
                Code::InvalidArgument,
                v1::ErrorReason::InvalidInput,
                "workflow request validation failed",
            ),
            StoreError::NotFound => self.status(
                Code::NotFound,
                v1::ErrorReason::ResourceNotFound,
                "workflow resource was not found",
            ),
            StoreError::IdempotencyConflict => self.status(
                Code::AlreadyExists,
                v1::ErrorReason::IdempotencyConflict,
                "idempotency key conflicts with the retained request",
            ),
            StoreError::Capacity(_) => self.status(
                Code::ResourceExhausted,
                v1::ErrorReason::BoundExceeded,
                "workflow capacity is exhausted",
            ),
            StoreError::WatchCursorExpired => self.status(
                Code::FailedPrecondition,
                v1::ErrorReason::WatchCursorExpired,
                "watch cursor expired; reconnect without a cursor for a snapshot",
            ),
            StoreError::HistorySnapshotExpired => self.status(
                Code::FailedPrecondition,
                v1::ErrorReason::HistorySnapshotExpired,
                "history snapshot expired",
            ),
            StoreError::CorruptState => self.status(
                Code::FailedPrecondition,
                v1::ErrorReason::FailedPrecondition,
                "retained workflow state is invalid",
            ),
            StoreError::ConfigurationRevisionConflict => self.status(
                Code::Aborted,
                v1::ErrorReason::FailedPrecondition,
                "configuration revision changed",
            ),
            StoreError::Database(_) => self.status(
                Code::Unavailable,
                v1::ErrorReason::Internal,
                "authoritative workflow state is unavailable",
            ),
        }
    }

    fn status(&self, code: Code, reason: v1::ErrorReason, message: &str) -> Status {
        let detail = v1::ErrorDetail {
            reason: reason as i32,
            message: message.to_owned(),
            resource_type: None,
            resource_id: None,
            metadata: Vec::new(),
            request_id: Some(self.request_id.clone()),
        };
        let rich = RichStatus {
            code: code as i32,
            message: message.to_owned(),
            details: vec![Any {
                type_url: ERROR_DETAIL_TYPE_URL.to_owned(),
                value: detail.encode_to_vec(),
            }],
        };
        let mut status = Status::with_details(code, message, rich.encode_to_vec().into());
        if let Ok(value) = self.request_id.parse() {
            status.metadata_mut().insert("x-request-id", value);
        }
        status
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct RichStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<Any>,
}

fn workspace(value: model::Workspace) -> v1::Workspace {
    v1::Workspace {
        id: value.id,
        display_name: value.display_name,
        created_at: Some(timestamp(value.created_at)),
        updated_at: Some(timestamp(value.updated_at)),
        resource_version: value.resource_version,
    }
}

fn project(value: model::Project) -> v1::Project {
    v1::Project {
        id: value.id,
        display_name: value.display_name,
        created_at: Some(timestamp(value.created_at)),
        updated_at: Some(timestamp(value.updated_at)),
        resource_version: value.resource_version,
        workspace_id: value.workspace_id,
        goal: value.goal,
    }
}

fn session(
    value: ayni_orchestrator_workflow_postgres::store::sessions::Session,
) -> v1::ConversationSession {
    v1::ConversationSession {
        id: value.id,
        workspace_id: value.workspace_id,
        project_id: value.project_id,
        conversation_id: value.conversation_id,
        generation: value.generation,
        state: value.state,
        idle_timeout_seconds: value.idle_timeout_seconds,
        can_replace: value.can_replace,
    }
}

fn conversation(value: model::Conversation) -> v1::Conversation {
    v1::Conversation {
        id: value.id,
        workspace_id: value.workspace_id,
        project_id: value.project_id,
        title: value.title,
        created_at: Some(timestamp(value.created_at)),
        revision: value.revision,
    }
}

fn receipt(value: model::CommandReceipt) -> v1::CommandReceipt {
    v1::CommandReceipt {
        command_id: value.command_id,
        kind: match value.kind {
            ReceiptKind::CreateWorkspace => v1::CommandReceiptKind::CreateWorkspace,
            ReceiptKind::CreateProject => v1::CommandReceiptKind::CreateProject,
            ReceiptKind::CreateConversation => v1::CommandReceiptKind::CreateConversation,
            ReceiptKind::SendMessage => v1::CommandReceiptKind::SendMessage,
        } as i32,
        workspace_id: value.workspace_id,
        project_id: value.project_id,
        conversation_id: value.conversation_id,
        message_id: value.message_id,
        turn_id: value.turn_id,
        accepted_at: Some(timestamp(value.accepted_at)),
        conversation_revision: value.conversation_revision,
        resume_cursor: value.resume_cursor,
        presentation_state: presentation(value.presentation_state),
    }
}

fn snapshot(value: model::ConversationSnapshot) -> v1::ConversationSnapshot {
    v1::ConversationSnapshot {
        workspace_id: value.workspace_id,
        project_id: value.project_id,
        conversation_id: value.conversation_id,
        conversation_revision: value.conversation_revision,
        recent_messages: value.recent_messages.into_iter().map(message).collect(),
        nonterminal_turns: value.nonterminal_turns.into_iter().map(turn).collect(),
        recent_terminal_turns: value
            .recent_terminal_turns
            .into_iter()
            .map(terminal)
            .collect(),
        history_counts: Some(v1::ConversationHistoryCounts {
            messages: value.message_count,
            turns: value.turn_count,
            has_older_history: value.has_older_history,
        }),
        history_snapshot_id: value.history_snapshot_id,
        current_activity: value.current_activity.map(activity),
        resume_cursor: value.resume_cursor,
        title: value.title,
        created_at: Some(timestamp(value.created_at)),
    }
}

fn message(value: model::Message) -> v1::Message {
    v1::Message {
        id: value.id,
        conversation_id: value.conversation_id,
        turn_id: value.turn_id,
        role: match value.role {
            MessageRole::User => v1::MessageRole::User,
            MessageRole::Assistant => v1::MessageRole::Assistant,
        } as i32,
        content: value.content,
        accepted_at: Some(timestamp(value.accepted_at)),
        sequence: value.sequence,
    }
}

fn turn(value: model::Turn) -> v1::Turn {
    v1::Turn {
        id: value.id,
        conversation_id: value.conversation_id,
        input_message_id: value.input_message_id,
        sequence: value.sequence,
        execution_binding_revision: value.execution_binding_revision,
        context_revision: value.context_revision,
        assignment_id: value.assignment_id,
        executor_generation: value.executor_generation,
        scheduling_state: match value.scheduling_state {
            SchedulingState::WaitingForPredecessor => v1::SchedulingState::WaitingForPredecessor,
            SchedulingState::Eligible => v1::SchedulingState::Eligible,
        } as i32,
        dispatch_state: match value.dispatch_state {
            DispatchState::NotEligible => v1::DispatchState::NotEligible,
            DispatchState::Pending => v1::DispatchState::Pending,
            DispatchState::Stored => v1::DispatchState::Stored,
            DispatchState::Transferred => v1::DispatchState::Transferred,
            DispatchState::Degraded => v1::DispatchState::Degraded,
        } as i32,
        execution_state: match value.execution_state {
            ExecutionState::NotStarted => v1::ExecutionState::NotStarted,
            ExecutionState::Processing => v1::ExecutionState::Processing,
            ExecutionState::Completed => v1::ExecutionState::Completed,
            ExecutionState::Failed => v1::ExecutionState::Failed,
        } as i32,
        response_message_id: value.response_message_id,
        failure_reason: value.failure_reason,
        accepted_at: Some(timestamp(value.accepted_at)),
        updated_at: Some(timestamp(value.updated_at)),
    }
}

fn activity(value: model::Activity) -> v1::Activity {
    v1::Activity {
        id: value.id,
        conversation_id: value.conversation_id,
        conversation_revision: value.conversation_revision,
        cursor: value.cursor,
        turn_id: value.turn_id,
        stage: match value.stage.as_str() {
            "admission" => v1::ActivityStage::Admission,
            "scheduling" => v1::ActivityStage::Scheduling,
            "dispatch" => v1::ActivityStage::Dispatch,
            "execution" => v1::ActivityStage::Execution,
            "terminal" => v1::ActivityStage::Terminal,
            "provisioning" => v1::ActivityStage::Provisioning,
            "agent_wakeup" => v1::ActivityStage::AgentWakeup,
            _ => v1::ActivityStage::Unspecified,
        } as i32,
        presentation_state: presentation(value.presentation_state),
        waiting_on: waiting_on(&value.waiting_on),
        safe_reason: value.safe_reason,
        accepted_at: Some(timestamp(value.accepted_at)),
        safe_detail: value.safe_detail,
    }
}

fn terminal(value: model::TerminalTurnSummary) -> v1::TerminalTurnSummary {
    let outcome = match value.presentation_state {
        PresentationState::Completed => Some(
            v1::terminal_turn_summary::Outcome::ResponseMessageId(value.response_message_id),
        ),
        PresentationState::Failed => Some(v1::terminal_turn_summary::Outcome::SafeFailureReason(
            value.safe_failure_reason,
        )),
        _ => None,
    };
    v1::TerminalTurnSummary {
        id: value.id,
        sequence: value.sequence,
        input_message_id: value.input_message_id,
        presentation_state: presentation(value.presentation_state),
        outcome,
        accepted_at: Some(timestamp(value.accepted_at)),
        terminal_at: Some(timestamp(value.terminal_at)),
    }
}

fn history_record(value: model::HistoryRecord) -> v1::ConversationHistoryRecord {
    let role_or_state = match (value.kind, value.role_or_state.as_str()) {
        (HistoryKind::Message, "user") => v1::ConversationHistoryRoleOrState::UserMessage,
        (HistoryKind::Message, "assistant") => v1::ConversationHistoryRoleOrState::AssistantMessage,
        (HistoryKind::TerminalTurn, "completed") => {
            v1::ConversationHistoryRoleOrState::CompletedTurn
        }
        (HistoryKind::TerminalTurn, "failed") => v1::ConversationHistoryRoleOrState::FailedTurn,
        _ => v1::ConversationHistoryRoleOrState::Unspecified,
    };
    v1::ConversationHistoryRecord {
        sequence: value.sequence,
        id: value.id,
        kind: match value.kind {
            HistoryKind::Message => v1::ConversationHistoryRecordKind::Message,
            HistoryKind::TerminalTurn => v1::ConversationHistoryRecordKind::TerminalTurn,
        } as i32,
        role_or_state: role_or_state as i32,
        content_or_outcome: value.content_or_outcome,
        source_turn_id: value.source_turn_id,
        accepted_at: Some(timestamp(value.accepted_at)),
    }
}

const fn presentation(value: PresentationState) -> i32 {
    (match value {
        PresentationState::Accepted => v1::PresentationState::Accepted,
        PresentationState::Queued => v1::PresentationState::Queued,
        PresentationState::Processing => v1::PresentationState::Processing,
        PresentationState::Completed => v1::PresentationState::Completed,
        PresentationState::Failed => v1::PresentationState::Failed,
    }) as i32
}

fn waiting_on(value: &str) -> i32 {
    (match value {
        "predecessor" => v1::WaitingOn::Predecessor,
        "delivery" => v1::WaitingOn::Delivery,
        "executor" => v1::WaitingOn::Executor,
        "provisioning" => v1::WaitingOn::Provisioning,
        _ => v1::WaitingOn::Unspecified,
    }) as i32
}

const fn timestamp(value: model::Timestamp) -> Timestamp {
    Timestamp {
        seconds: value.seconds,
        nanos: value.nanos,
    }
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn update(value: ConversationUpdate) -> Result<v1::ConversationUpdate, StoreError> {
    let kind = match (value.kind.as_str(), value.presentation_state) {
        ("conversation_created", _) => v1::ConversationUpdateKind::ConversationCreated,
        ("message_admitted", _) => v1::ConversationUpdateKind::MessageAdmitted,
        ("turn_state_changed", PresentationState::Completed | PresentationState::Failed) => {
            v1::ConversationUpdateKind::TerminalOutcomeAccepted
        }
        ("turn_state_changed", PresentationState::Queued) => {
            v1::ConversationUpdateKind::TurnSchedulingChanged
        }
        ("turn_state_changed", _) => v1::ConversationUpdateKind::TurnExecutionChanged,
        _ => return Err(StoreError::CorruptState),
    };
    Ok(v1::ConversationUpdate {
        update_id: value.update_id,
        activity_id: value.activity_id,
        cursor: value.cursor,
        conversation_revision: value.conversation_revision,
        kind: kind as i32,
        affected_resource_ids: value.affected_resource_ids,
        presentation_state: presentation(value.presentation_state),
        waiting_on: waiting_on(&value.waiting_on),
        safe_reason: value.safe_reason,
        accepted_at: Some(timestamp(value.accepted_at)),
    })
}

#[cfg(test)]
mod observation_status_tests;

fn agent_configuration(value: model::AgentConfiguration) -> v1::AgentConfiguration {
    v1::AgentConfiguration {
        workspace_id: value.workspace_id,
        revision_id: value.revision_id,
        instructions: value.instructions,
        provider: value.provider,
        model: value.model,
        max_input_tokens: value.max_input_tokens,
        max_output_tokens: value.max_output_tokens,
        max_total_tokens: value.max_total_tokens,
        idle_timeout_seconds: value.idle_timeout_seconds,
    }
}
