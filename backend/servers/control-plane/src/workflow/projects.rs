//! Project and workspace commands and discovery.
use super::projections::{agent_configuration, project, receipt, workspace};
use super::{RequestContext, WorkflowGrpc};
use ayni_orchestrator_api::{v1, validation};
use ayni_orchestrator_workflow_postgres::model;
use tonic::{Code, Request, Response, Status};

#[tonic::async_trait]
impl v1::project_service_server::ProjectService for WorkflowGrpc {
    async fn rename_workspace(
        &self,
        request: Request<v1::RenameWorkspaceRequest>,
    ) -> Result<Response<v1::RenameWorkspaceResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let command_id = self
            .store
            .rename_workspace(
                &value.idempotency_key,
                &value.workspace_id,
                value.expected_resource_version,
                &value.display_name,
            )
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::RenameWorkspaceResponse { command_id }))
    }
    async fn rename_project(
        &self,
        request: Request<v1::RenameProjectRequest>,
    ) -> Result<Response<v1::RenameProjectResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let command_id = self
            .store
            .rename_project(
                &value.idempotency_key,
                &value.workspace_id,
                &value.project_id,
                value.expected_resource_version,
                &value.display_name,
            )
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::RenameProjectResponse { command_id }))
    }

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
            connection_id: proposed.connection_id,
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
            command_event_id: result.command_event_id,
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
