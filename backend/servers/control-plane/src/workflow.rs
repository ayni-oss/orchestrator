mod plans;
mod projections;
use projections::{
    agent_configuration, conversation, history_record, project, receipt, session, snapshot,
    timestamp, update, valid_request_id, workspace,
};
mod errors;
use errors::RequestContext;
use prost::Message as _;

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
use std::{io, net::SocketAddr, pin::Pin};
use tokio::sync::watch;
use tokio_stream::{Stream, StreamExt as _};
use tonic::{Code, Request, Response, Status, transport::Server};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 3 * 1024 * 1024;
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

/// Serve the trusted-loopback workflow API.
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
    async fn approve_plan(
        &self,
        request: Request<v1::ApprovePlanRequest>,
    ) -> Result<Response<v1::ApprovePlanResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let scope = plans::scope(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
        );
        let result = self
            .store
            .approve_plan(
                scope,
                &value.idempotency_key,
                &value.plan_id,
                &value.expected_version_id,
                &value.content_digest,
            )
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(plans::approved(result)))
    }

    async fn get_plan_approval(
        &self,
        request: Request<v1::GetPlanApprovalRequest>,
    ) -> Result<Response<v1::GetPlanApprovalResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let scope = plans::scope(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
        );
        let result = self
            .store
            .get_plan_approval(scope, &value.plan_id)
            .await
            .map_err(|error| context.store(&error))?;
        let result = result.map(plans::approved).unwrap_or_default();
        Ok(context.response(v1::GetPlanApprovalResponse {
            approval: result.approval,
            tasks: result.tasks,
        }))
    }

    async fn list_tasks(
        &self,
        request: Request<v1::ListTasksRequest>,
    ) -> Result<Response<v1::ListTasksResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let scope = plans::scope(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
        );
        let result = self
            .store
            .list_tasks(scope, &value.after_id, value.limit)
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::ListTasksResponse {
            tasks: result.tasks.into_iter().map(plans::task).collect(),
            next_after_id: result.next_after_id,
        }))
    }

    async fn get_task(
        &self,
        request: Request<v1::GetTaskRequest>,
    ) -> Result<Response<v1::GetTaskResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let scope = plans::scope(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
        );
        let result = self
            .store
            .get_task(scope, &value.task_id)
            .await
            .map_err(|error| context.store(&error))?;
        Ok(context.response(v1::GetTaskResponse {
            task: Some(plans::task(result)),
        }))
    }

    async fn create_plan(
        &self,
        request: Request<v1::CreatePlanRequest>,
    ) -> Result<Response<v1::CreatePlanResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let scope = plans::scope(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
        );
        Ok(context.response(plans::written(
            self.store
                .create_plan(
                    scope,
                    &value.idempotency_key,
                    &value.follows_plan_id,
                    &plans::content(value.content, &context)?,
                )
                .await
                .map_err(|e| context.store(&e))?,
        )))
    }
    async fn revise_plan(
        &self,
        request: Request<v1::RevisePlanRequest>,
    ) -> Result<Response<v1::RevisePlanResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let scope = plans::scope(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
        );
        Ok(context.response(plans::revised(
            self.store
                .revise_plan(
                    scope,
                    &value.idempotency_key,
                    &value.plan_id,
                    &value.expected_version_id,
                    &plans::content(value.content, &context)?,
                )
                .await
                .map_err(|e| context.store(&e))?,
        )))
    }
    async fn get_plan_version(
        &self,
        request: Request<v1::GetPlanVersionRequest>,
    ) -> Result<Response<v1::GetPlanVersionResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let scope = plans::scope(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
        );
        Ok(context.response(v1::GetPlanVersionResponse {
            version: Some(plans::version(
                self.store
                    .get_plan_version(scope, &value.plan_id, &value.version_id)
                    .await
                    .map_err(|e| context.store(&e))?,
            )),
        }))
    }
    async fn list_plans(
        &self,
        request: Request<v1::ListPlansRequest>,
    ) -> Result<Response<v1::ListPlansResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let scope = plans::scope(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
        );
        Ok(context.response(v1::ListPlansResponse {
            plans: self
                .store
                .list_plans(scope)
                .await
                .map_err(|e| context.store(&e))?
                .into_iter()
                .map(plans::summary)
                .collect(),
        }))
    }
    async fn list_plan_versions(
        &self,
        request: Request<v1::ListPlanVersionsRequest>,
    ) -> Result<Response<v1::ListPlanVersionsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let scope = plans::scope(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
        );
        Ok(context.response(plans::page(
            self.store
                .list_plan_versions(scope, &value.plan_id, value.before_ordinal, value.limit)
                .await
                .map_err(|e| context.store(&e))?,
        )))
    }
    async fn list_provider_calls(
        &self,
        request: Request<v1::ListProviderCallsRequest>,
    ) -> Result<Response<v1::ListProviderCallsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let calls = self
            .store
            .list_provider_calls(
                &value.workspace_id,
                &value.project_id,
                &value.conversation_id,
            )
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::ListProviderCallsResponse {
            calls: calls
                .into_iter()
                .map(|call| {
                    let evidence = call.evidence;
                    v1::ProviderCall {
                        turn_id: call.turn_id,
                        call_id: evidence.call_id,
                        provider: evidence.provider,
                        model: evidence.model,
                        state: call.state,
                        input_tokens: evidence.input_tokens,
                        output_tokens: evidence.output_tokens,
                        total_tokens: evidence.total_tokens,
                        reserved_input_tokens: evidence.reserved_input_tokens,
                        reserved_output_tokens: evidence.reserved_output_tokens,
                        reserved_total_tokens: evidence.reserved_total_tokens,
                        cancellation_requested: call.cancellation_requested,
                        uncertainty_acknowledged: call.uncertainty_acknowledged,
                        safe_reason: call.safe_reason,
                        response_id: evidence.response_id,
                        reported_model: evidence.reported_model,
                        request_digest: evidence.request_digest,
                        context_digest: evidence.context_digest,
                        configuration_revision_id: evidence.configuration_revision_id,
                        tool_invocations: evidence
                            .tool_invocations
                            .into_iter()
                            .map(|tool| v1::ToolInvocation {
                                invocation_id: tool.invocation_id,
                                name: tool.name,
                                input_digest: tool.input_digest,
                                output_digest: tool.output_digest,
                                state: tool.state,
                            })
                            .collect(),
                    }
                })
                .collect(),
        }))
    }
    async fn acknowledge_provider_uncertainty(
        &self,
        request: Request<v1::AcknowledgeProviderUncertaintyRequest>,
    ) -> Result<Response<v1::AcknowledgeProviderUncertaintyResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let result = self
            .store
            .acknowledge_provider_uncertainty(
                &value.workspace_id,
                &value.project_id,
                &value.conversation_id,
                &value.turn_id,
                &value.call_id,
                &value.request_digest,
                &value.idempotency_key,
                value.acknowledge_possible_charge,
            )
            .await
            .map_err(|e| context.store(&e))?;
        Ok(
            context.response(v1::AcknowledgeProviderUncertaintyResponse {
                command_id: result.command_id,
                turn_id: result.turn_id,
                call_id: result.call_id,
                state: "acknowledged".into(),
            }),
        )
    }

    async fn cancel_turn(
        &self,
        request: Request<v1::CancelTurnRequest>,
    ) -> Result<Response<v1::CancelTurnResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let result = self
            .store
            .cancel_turn(
                &value.workspace_id,
                &value.project_id,
                &value.conversation_id,
                &value.turn_id,
                &value.idempotency_key,
            )
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::CancelTurnResponse {
            command_id: result.command_id,
            turn_id: result.turn_id,
            state: result.state,
            cancellation_requested: result.cancellation_requested,
        }))
    }

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

#[cfg(test)]
mod observation_status_tests;
