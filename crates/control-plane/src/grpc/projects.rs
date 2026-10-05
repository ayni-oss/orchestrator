use super::{GrpcApi, Request, Response, Status, api_validation, page_response, v1, with_store};

#[tonic::async_trait]
impl v1::project_service_server::ProjectService for GrpcApi {
    async fn create_project(
        &self,
        request: Request<v1::CreateProjectRequest>,
    ) -> Result<Response<v1::CreateProjectResponse>, Status> {
        let context = self.context(&request, "CreateProject")?;
        api_validation::create_project(request.get_ref()).map_err(|e| context.validation(&e))?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let project = with_store(&self.state, |s| s.create_project(&key, request.get_ref()))
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
        let page = with_store(&self.state, |s| {
            s.list_projects(value.state, value.page.as_ref())
        })
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
        let project = with_store(&self.state, |s| s.project(&request.get_ref().project_id))
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
        .map_err(|e| context.validation(&e))?;
        let key = context.idempotency_key(&value.idempotency_key)?;
        let project = with_store(&self.state, |s| s.update_project(&key, value))
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
        let project = with_store(&self.state, |s| s.archive_project(&key, value))
            .map_err(|e| context.store_error(&e))?;
        Ok(context.response(v1::ArchiveProjectResponse {
            project: Some(project),
        }))
    }
}
