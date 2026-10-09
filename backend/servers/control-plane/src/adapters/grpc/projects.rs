use super::{GrpcApi, Request, Response, Status, api_validation, page_response, v1, with_store};

#[tonic::async_trait]
impl v1::project_service_server::ProjectService for GrpcApi {
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
        .and_then(|()| {
            if value.repository_connection_ids.is_empty() {
                Ok(())
            } else {
                api_validation::project_members(&value.repository_connection_ids)
            }
        })
        .map_err(|e| context.validation(&e))?;
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
