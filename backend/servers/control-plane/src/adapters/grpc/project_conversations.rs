//! Trusted operator transport for research and explicit task approvals.
use super::{GrpcApi, Request, Response, Status, v1, with_store};
use uuid::Uuid;

#[tonic::async_trait]
impl v1::project_conversation_service_server::ProjectConversationService for GrpcApi {
    async fn get_project_conversation_context(
        &self,
        request: Request<v1::GetProjectConversationContextRequest>,
    ) -> Result<Response<v1::GetProjectConversationContextResponse>, Status> {
        let context = self.context(&request, "GetProjectConversationContext")?;
        let message = request.into_inner();
        let result = with_store(&self.state, move |s| {
            s.project_conversation_context(&message.project_id, &message.conversation_id)
        })
        .await
        .map_err(|e| context.store_error(&e))?;
        Ok(context.response(result))
    }
    async fn list_project_conversation_history(
        &self,
        request: Request<v1::ListProjectConversationHistoryRequest>,
    ) -> Result<Response<v1::ListProjectConversationHistoryResponse>, Status> {
        let context = self.context(&request, "ListProjectConversationHistory")?;
        let message = request.into_inner();
        let result = with_store(&self.state, move |s| {
            s.project_conversation_history(&message)
        })
        .await
        .map_err(|e| context.store_error(&e))?;
        Ok(context.response(result))
    }
    type WatchProjectConversationStream =
        super::BoxGrpcStream<v1::WatchProjectConversationResponse>;
    async fn watch_project_conversation(
        &self,
        request: Request<v1::WatchProjectConversationRequest>,
    ) -> Result<Response<Self::WatchProjectConversationStream>, Status> {
        let context = self.context(&request, "WatchProjectConversation")?;
        let message = request.into_inner();
        let state = self.state.clone();
        let mut shutdown = self.shutdown.clone();
        let stream = async_stream::try_stream! {
            let mut cursor=message.after_cursor;
            let mut interval=tokio::time::interval(std::time::Duration::from_secs(2));
            loop{
                if *shutdown.borrow(){break}
                tokio::select!{_ = shutdown.changed()=>break,_ = interval.tick()=>{}}
                let id=message.conversation_id.clone();
                let c=with_store(&state,move|s|s.project_conversation(&id)).await.map_err(|e|context.store_error(&e))?;
                if c.update_cursor>cursor{cursor=c.update_cursor;yield v1::WatchProjectConversationResponse{conversation:Some(c)};}
            }
        };
        Ok(Response::new(Box::pin(stream)))
    }
    async fn create_project_conversation(
        &self,
        request: Request<v1::CreateProjectConversationRequest>,
    ) -> Result<Response<v1::CreateProjectConversationResponse>, Status> {
        let context = self.context(&request, "CreateProjectConversation")?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let c = with_store(&self.state, move |s| {
            s.create_project_conversation(&key, &message)
        })
        .await
        .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::CreateProjectConversationResponse {
            conversation: Some(c),
        }))
    }
    async fn list_project_conversations(
        &self,
        request: Request<v1::ListProjectConversationsRequest>,
    ) -> Result<Response<v1::ListProjectConversationsResponse>, Status> {
        let context = self.context(&request, "ListProjectConversations")?;
        let message = request.into_inner();
        let values = with_store(&self.state, move |s| {
            s.list_project_conversations(&message.project_id)
        })
        .await
        .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::ListProjectConversationsResponse {
            conversations: values,
        }))
    }
    async fn get_project_conversation(
        &self,
        request: Request<v1::GetProjectConversationRequest>,
    ) -> Result<Response<v1::GetProjectConversationResponse>, Status> {
        let context = self.context(&request, "GetProjectConversation")?;
        let message = request.into_inner();
        let c = with_store(&self.state, move |s| {
            s.project_conversation(&message.conversation_id)
        })
        .await
        .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::GetProjectConversationResponse {
            conversation: Some(c),
        }))
    }
    async fn send_project_message(
        &self,
        request: Request<v1::SendProjectMessageRequest>,
    ) -> Result<Response<v1::SendProjectMessageResponse>, Status> {
        let context = self.context(&request, "SendProjectMessage")?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let replay_key = key.clone();
        let replay_message = message.clone();
        if let Some(c) = with_store(&self.state, move |s| {
            s.replay_conversation(&replay_key, &replay_message)
        })
        .await
        .map_err(|e| context.store_error(&e))?
        {
            return Ok(context.response(v1::SendProjectMessageResponse {
                conversation: Some(c),
            }));
        }
        let id = message.conversation_id.clone();
        let c = with_store(&self.state, move |s| s.project_conversation(&id))
            .await
            .map_err(|e| context.store_error(&e))?;
        let project = with_store(&self.state, move |s| s.project(&c.project_id))
            .await
            .map_err(|e| context.store_error(&e))?;
        let defaults = with_store(&self.state, move |s| s.effective_project_defaults(&project))
            .await
            .map_err(|e| context.store_error(&e))?
            .ok_or_else(|| {
                Status::failed_precondition(
                    "select project agent connection and model in project settings",
                )
            })?;
        let _connection = self.state.connection_lock.lock().await;
        let binding = self
            .state
            .freeze_provider_credentials(&defaults.project_connection_id, &defaults.project_model)
            .await?;
        let run = v1::ProjectAgentRun {
            id: format!("run-{}", Uuid::now_v7()),
            state: "queued".into(),
            model: defaults.project_model,
            connection_id: binding.connection_id.clone(),
            credential_revision: binding.credential_revision.clone(),
            provider_credentials: Some(binding),
            purpose: "research".into(),
            created_at: Some(prost_types::Timestamp {
                seconds: crate::adapters::sqlite::unix_seconds()
                    .map_err(|e| context.store_error(&e))?,
                nanos: 0,
            }),
            ..Default::default()
        };
        let c = with_store(&self.state, move |s| {
            s.send_project_message(&key, &message, run)
        })
        .await
        .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::SendProjectMessageResponse {
            conversation: Some(c),
        }))
    }
    async fn decide_project_plan(
        &self,
        request: Request<v1::DecideProjectPlanRequest>,
    ) -> Result<Response<v1::DecideProjectPlanResponse>, Status> {
        let context = self.context(&request, "DecideProjectPlan")?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let replay_key = key.clone();
        let replay_message = message.clone();
        if let Some(c) = with_store(&self.state, move |s| {
            s.replay_conversation(&replay_key, &replay_message)
        })
        .await
        .map_err(|e| context.store_error(&e))?
        {
            return Ok(context.response(v1::DecideProjectPlanResponse {
                conversation: Some(c),
            }));
        }
        let lifecycle = self.state.repository_lifecycle.read().await;
        let _connection = self.state.connection_lock.lock().await;
        if message.decision == "approve" {
            let id = message.conversation_id.clone();
            let c = with_store(&self.state, move |s| s.project_conversation(&id))
                .await
                .map_err(|e| context.store_error(&e))?;
            let plan = c
                .plan
                .as_ref()
                .ok_or_else(|| Status::not_found("plan not found"))?;
            if plan.id != message.plan_id {
                return Err(Status::not_found("plan not found"));
            }
            let version = plan
                .versions
                .iter()
                .find(|v| v.version == message.version)
                .ok_or_else(|| {
                    Status::aborted("plan version changed; review the latest version")
                })?;
            if version.state == "pending" {
                if plan
                    .versions
                    .last()
                    .is_none_or(|v| v.version != message.version)
                {
                    return Err(Status::aborted(
                        "plan version changed; review the latest version",
                    ));
                }
                for proposal in c
                    .proposals
                    .iter()
                    .filter(|p| version.proposal_ids.contains(&p.id) && p.state != "approved")
                {
                    let requested = proposal
                        .request
                        .as_ref()
                        .ok_or_else(|| Status::internal("proposal invalid"))?;
                    let observed = self
                        .state
                        .observe_project_repository_head_guarded(
                            &c.project_id,
                            &requested.repository_connection_id,
                            &lifecycle,
                        )
                        .await
                        .map_err(Status::failed_precondition)?;
                    if observed.revision != requested.repository_revision {
                        return Err(Status::aborted(
                            "repository source changed; request a revised plan and review its new version",
                        ));
                    }
                    let model = requested
                        .agent
                        .as_ref()
                        .ok_or_else(|| Status::failed_precondition("coding model required"))?;
                    self.state
                        .freeze_provider_credentials(&requested.connection_id, &model.model)
                        .await?;
                }
            }
        }
        let c = with_store(&self.state, move |s| s.decide_project_plan(&key, &message))
            .await
            .map_err(|e| match e {
            crate::adapters::sqlite::CleanStoreError::StaleVersion => Status::aborted("plan, source, or runtime changed; request a revised plan and review its new version"),
            crate::adapters::sqlite::CleanStoreError::ProfileNotReady => Status::failed_precondition("runtime is no longer ready; restore readiness and request a revised plan"),
            _ => context.store_error(&e),
        })?;
        drop(lifecycle);
        Ok(context.response(v1::DecideProjectPlanResponse {
            conversation: Some(c),
        }))
    }
    async fn decide_project_proposal(
        &self,
        request: Request<v1::DecideProjectProposalRequest>,
    ) -> Result<Response<v1::DecideProjectProposalResponse>, Status> {
        let context = self.context(&request, "DecideProjectProposal")?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let replay_key = key.clone();
        let replay_message = message.clone();
        if let Some(c) = with_store(&self.state, move |s| {
            s.replay_conversation(&replay_key, &replay_message)
        })
        .await
        .map_err(|e| context.store_error(&e))?
        {
            return Ok(context.response(v1::DecideProjectProposalResponse {
                conversation: Some(c),
            }));
        }
        let lifecycle = self.state.repository_lifecycle.read().await;
        let _connection = self.state.connection_lock.lock().await;
        if message.decision == "approve" {
            let id = message.conversation_id.clone();
            let c = with_store(&self.state, move |s| s.project_conversation(&id))
                .await
                .map_err(|e| context.store_error(&e))?;
            let proposal = c
                .proposals
                .iter()
                .find(|p| p.id == message.proposal_id)
                .ok_or_else(|| Status::not_found("proposal not found"))?;
            if proposal.state == "pending" && proposal.action == "create_task" {
                return Err(Status::failed_precondition(
                    "review the exact plan version and use DecideProjectPlan before approving tasks",
                ));
            }
        }
        let c = with_store(&self.state, move |s| {
            s.decide_project_proposal(&key, &message)
        })
        .await
        .map_err(|e| context.store_error(&e))?;
        drop(lifecycle);
        Ok(context.response(v1::DecideProjectProposalResponse {
            conversation: Some(c),
        }))
    }
}
