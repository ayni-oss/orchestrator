use super::{GrpcApi, Request, Response, Status, api_validation, page_response, v1, with_store};

#[tonic::async_trait]
impl v1::project_service_server::ProjectService for GrpcApi {
    async fn create_workspace(
        &self,
        request: Request<v1::CreateWorkspaceRequest>,
    ) -> Result<Response<v1::CreateWorkspaceResponse>, Status> {
        let context = self.context(&request, "CreateWorkspace")?;
        let value = request.get_ref();
        api_validation::project_name(&value.display_name)
            .and_then(|()| api_validation::project_members(&value.repository_connection_ids))
            .map_err(|e| context.validation(&e))?;
        let key = context.idempotency_key(&value.idempotency_key)?;
        let replay_request = request.get_ref().clone();
        let replay_key = key.clone();
        if let Some(workspace) = with_store(&self.state, move |store| {
            store.replay_workspace("CreateWorkspace", &replay_key, &replay_request)
        })
        .await
        .map_err(|e| context.store_error(&e))?
        {
            return Ok(context.response(v1::CreateWorkspaceResponse {
                workspace: Some(workspace),
            }));
        }
        let _guard = self.state.connection_lock.lock().await;
        self.validate_workspace_provider_defaults(value.agent_defaults.as_ref())
            .await?;
        let message = request.into_inner();
        let workspace = with_store(&self.state, move |s| s.create_workspace(&key, &message))
            .await
            .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::CreateWorkspaceResponse {
            workspace: Some(workspace),
        }))
    }
    async fn list_workspaces(
        &self,
        request: Request<v1::ListWorkspacesRequest>,
    ) -> Result<Response<v1::ListWorkspacesResponse>, Status> {
        let context = self.context(&request, "ListWorkspaces")?;
        let value = request.get_ref();
        api_validation::page(value.page.as_ref())
            .and_then(|()| api_validation::enum_filter(value.state, "state", &[1, 2]))
            .map_err(|e| context.validation(&e))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |s| {
            s.list_workspaces(message.state, message.page.as_ref())
        })
        .await
        .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::ListWorkspacesResponse {
            workspaces: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }
    async fn get_workspace(
        &self,
        request: Request<v1::GetWorkspaceRequest>,
    ) -> Result<Response<v1::GetWorkspaceResponse>, Status> {
        let context = self.context(&request, "GetWorkspace")?;
        api_validation::resource_id(&request.get_ref().workspace_id, "workspace_id")
            .map_err(|e| context.validation(&e))?;
        let message = request.into_inner();
        let workspace = with_store(&self.state, move |s| s.workspace(&message.workspace_id))
            .await
            .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::GetWorkspaceResponse {
            workspace: Some(workspace),
        }))
    }
    async fn update_workspace(
        &self,
        request: Request<v1::UpdateWorkspaceRequest>,
    ) -> Result<Response<v1::UpdateWorkspaceResponse>, Status> {
        let context = self.context(&request, "UpdateWorkspace")?;
        let value = request.get_ref();
        api_validation::project_mutation(
            &value.idempotency_key,
            &value.workspace_id,
            value.expected_resource_version,
        )
        .and_then(|()| api_validation::project_name(&value.display_name))
        .and_then(|()| {
            if value.repository_connection_ids.is_empty() {
                Ok(())
            } else {
                api_validation::project_members(&value.repository_connection_ids)
            }
        })
        .map_err(|e| context.validation(&e))?;
        let key = context.idempotency_key(&value.idempotency_key)?;
        let replay_request = request.get_ref().clone();
        let replay_key = key.clone();
        if let Some(workspace) = with_store(&self.state, move |store| {
            store.replay_workspace("UpdateWorkspace", &replay_key, &replay_request)
        })
        .await
        .map_err(|e| context.store_error(&e))?
        {
            return Ok(context.response(v1::UpdateWorkspaceResponse {
                workspace: Some(workspace),
            }));
        }
        let _guard = self.state.connection_lock.lock().await;
        self.validate_workspace_provider_defaults(value.agent_defaults.as_ref())
            .await?;
        let message = request.into_inner();
        let workspace = with_store(&self.state, move |s| s.update_workspace(&key, &message))
            .await
            .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::UpdateWorkspaceResponse {
            workspace: Some(workspace),
        }))
    }
    async fn archive_workspace(
        &self,
        request: Request<v1::ArchiveWorkspaceRequest>,
    ) -> Result<Response<v1::ArchiveWorkspaceResponse>, Status> {
        let context = self.context(&request, "ArchiveWorkspace")?;
        let value = request.get_ref();
        api_validation::project_mutation(
            &value.idempotency_key,
            &value.workspace_id,
            value.expected_resource_version,
        )
        .map_err(|e| context.validation(&e))?;
        let key = context.idempotency_key(&value.idempotency_key)?;
        let message = request.into_inner();
        let workspace = with_store(&self.state, move |s| s.archive_workspace(&key, &message))
            .await
            .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::ArchiveWorkspaceResponse {
            workspace: Some(workspace),
        }))
    }
    type WatchProjectsStream = super::BoxGrpcStream<v1::WatchProjectsResponse>;
    async fn watch_projects(
        &self,
        request: Request<v1::WatchProjectsRequest>,
    ) -> Result<Response<Self::WatchProjectsStream>, Status> {
        let context = self.context(&request, "WatchProjects")?;
        Ok(context.response(Box::pin(super::resource_watch::projects(
            self.state.clone(),
            self.shutdown.clone(),
            request.into_inner(),
            context.request_id.clone(),
        ))))
    }

    async fn list_project_summaries(
        &self,
        request: Request<v1::ListProjectSummariesRequest>,
    ) -> Result<Response<v1::ListProjectSummariesResponse>, Status> {
        let context = self.context(&request, "ListProjectSummaries")?;
        api_validation::page(request.get_ref().page.as_ref())
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store.project_summaries(message.page.as_ref())
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListProjectSummariesResponse {
            summaries: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn create_project(
        &self,
        request: Request<v1::CreateProjectRequest>,
    ) -> Result<Response<v1::CreateProjectResponse>, Status> {
        let context = self.context(&request, "CreateProject")?;
        api_validation::create_project(request.get_ref()).map_err(|e| context.validation(&e))?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let project = with_store(&self.state, move |s| s.create_project(&key, &message))
            .await
            .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::CreateProjectResponse {
            project: Some(project),
        }))
    }

    async fn list_projects(
        &self,
        request: Request<v1::ListProjectsRequest>,
    ) -> Result<Response<v1::ListProjectsResponse>, Status> {
        let context = self.context(&request, "ListProjects")?;
        let value = request.get_ref();
        api_validation::page(value.page.as_ref())
            .and_then(|()| {
                api_validation::enum_filter(
                    value.state,
                    "state",
                    &[
                        v1::ProjectState::Active as i32,
                        v1::ProjectState::Archived as i32,
                    ],
                )
            })
            .map_err(|e| context.validation(&e))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |s| {
            s.list_projects(message.state, message.page.as_ref())
        })
        .await
        .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::ListProjectsResponse {
            projects: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_project(
        &self,
        request: Request<v1::GetProjectRequest>,
    ) -> Result<Response<v1::GetProjectResponse>, Status> {
        let context = self.context(&request, "GetProject")?;
        api_validation::resource_id(&request.get_ref().project_id, "project_id")
            .map_err(|e| context.validation(&e))?;
        let message = request.into_inner();
        let project = with_store(&self.state, move |s| s.project(&message.project_id))
            .await
            .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::GetProjectResponse {
            project: Some(project),
        }))
    }

    async fn update_project(
        &self,
        request: Request<v1::UpdateProjectRequest>,
    ) -> Result<Response<v1::UpdateProjectResponse>, Status> {
        let context = self.context(&request, "UpdateProject")?;
        let value = request.get_ref();
        api_validation::project_mutation(
            &value.idempotency_key,
            &value.project_id,
            value.expected_resource_version,
        )
        .and_then(|()| api_validation::project_name(&value.display_name))
        .and_then(|()| api_validation::project_goal(value.goal.as_deref().unwrap_or_default()))
        .and_then(|()| {
            if value.repository_connection_ids.is_empty() {
                Ok(())
            } else {
                api_validation::project_members(&value.repository_connection_ids)
            }
        })
        .map_err(|e| context.validation(&e))?;
        let _connection_guard = self.state.connection_lock.lock().await;
        if let Some(defaults) = value.agent_defaults.as_ref() {
            for (connection, model) in [
                (&defaults.project_connection_id, &defaults.project_model),
                (&defaults.coding_connection_id, &defaults.coding_model),
            ] {
                if connection.is_empty() && model.is_empty() {
                    continue;
                }
                api_validation::resource_id(connection, "connection_id")
                    .map_err(|error| context.validation(&error))?;
                if model.is_empty() {
                    return Err(Status::invalid_argument("provider_model_required"));
                }
            }
            let project_validation = async {
                if !defaults.project_connection_id.is_empty() {
                    self.state
                        .freeze_provider_credentials(
                            &defaults.project_connection_id,
                            &defaults.project_model,
                        )
                        .await?;
                }
                Ok::<(), Status>(())
            };
            let coding_validation = async {
                if !defaults.coding_connection_id.is_empty() {
                    self.state
                        .freeze_provider_credentials(
                            &defaults.coding_connection_id,
                            &defaults.coding_model,
                        )
                        .await?;
                }
                Ok::<(), Status>(())
            };
            tokio::try_join!(project_validation, coding_validation)?;
        }
        let key = context.idempotency_key(&value.idempotency_key)?;
        let message = request.into_inner();
        let project = with_store(&self.state, move |s| s.update_project(&key, &message))
            .await
            .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::UpdateProjectResponse {
            project: Some(project),
        }))
    }

    async fn archive_project(
        &self,
        request: Request<v1::ArchiveProjectRequest>,
    ) -> Result<Response<v1::ArchiveProjectResponse>, Status> {
        let context = self.context(&request, "ArchiveProject")?;
        let value = request.get_ref();
        api_validation::project_mutation(
            &value.idempotency_key,
            &value.project_id,
            value.expected_resource_version,
        )
        .map_err(|e| context.validation(&e))?;
        let key = context.idempotency_key(&value.idempotency_key)?;
        let message = request.into_inner();
        let project = with_store(&self.state, move |s| s.archive_project(&key, &message))
            .await
            .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::ArchiveProjectResponse {
            project: Some(project),
        }))
    }
}

impl GrpcApi {
    async fn validate_workspace_provider_defaults(
        &self,
        defaults: Option<&v1::ProjectAgentDefaults>,
    ) -> Result<(), Status> {
        if let Some(defaults) = defaults {
            for (connection, model) in [
                (&defaults.project_connection_id, &defaults.project_model),
                (&defaults.coding_connection_id, &defaults.coding_model),
            ] {
                if connection.is_empty() && model.is_empty() {
                    continue;
                }
                if model.is_empty() || model.len() > 128 || model.chars().any(char::is_control) {
                    return Err(Status::invalid_argument("provider_model_required"));
                }
                self.state
                    .freeze_provider_credentials(connection, model)
                    .await?;
            }
        }
        Ok(())
    }
}
