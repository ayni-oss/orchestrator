//! Local-operator source discovery, durable catalog admission and project association.
mod github;
use super::{RequestContext, WorkflowGrpc, v1};
use ayni_orchestrator_workflow_postgres::{
    StoreError,
    store::repositories::{self, Connect, Operation},
};
use std::time::Duration;
use tonic::{Request, Response, Status};
fn repository(value: repositories::Repository) -> v1::Repository {
    v1::Repository {
        id: value.id,
        connection_id: value.connection_id,
        installation_id: value.installation_id,
        provider_repository_id: value.provider_repository_id,
        full_name: value.full_name,
        default_branch: value.default_branch,
        private: value.private,
        connected_at_unix_ms: value.connected_at_unix_ms,
    }
}
fn operation(value: Operation) -> v1::RepositoryOperation {
    v1::RepositoryOperation {
        id: value.id,
        idempotency_key: value.idempotency_key,
        connection_id: value.connection_id,
        connection_revision: value.connection_revision,
        installation_id: value.installation_id,
        provider_repository_id: value.provider_repository_id,
        state: value.state,
        safe_reason: value.safe_reason,
        repository: value.repository.map(repository),
        accepted_at_unix_ms: value.accepted_at_unix_ms,
        updated_at_unix_ms: value.updated_at_unix_ms,
    }
}
fn page(value: u32) -> Result<u32, Status> {
    if value > 1000 {
        return Err(Status::invalid_argument(
            "discovery page exceeds supported bounds",
        ));
    }
    Ok(value.max(1))
}
fn identity(value: u64) -> Result<(), Status> {
    if value == 0 || i64::try_from(value).is_err() {
        return Err(Status::invalid_argument("invalid provider identity"));
    }
    Ok(())
}
#[tonic::async_trait]
impl v1::repository_service_server::RepositoryService for WorkflowGrpc {
    async fn list_github_installations(
        &self,
        request: Request<v1::ListGithubInstallationsRequest>,
    ) -> Result<Response<v1::ListGithubInstallationsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let page = page(value.page)?;
        let source = self
            .store
            .repository_source(&value.connection_id, value.expected_connection_revision)
            .await
            .map_err(|e| context.store(&e))?;
        let result = match github::Github::new(&source) {
            Ok(client) => client.installations(page).await,
            Err(reason) => Err(reason),
        };
        // A rotated/disabled source cannot return a newly accepted observation.
        self.store
            .repository_source(&value.connection_id, value.expected_connection_revision)
            .await
            .map_err(|e| context.store(&e))?;
        let response = match result {
            Ok(items) => v1::ListGithubInstallationsResponse {
                next_page: if items.len() == 100 && page < 1000 {
                    page + 1
                } else {
                    0
                },
                installations: items
                    .into_iter()
                    .map(|v| v1::GithubInstallation {
                        installation_id: v.id,
                        account_login: v.login,
                        suspended: v.suspended,
                    })
                    .collect(),
                safe_reason: String::new(),
            },
            Err(reason) => v1::ListGithubInstallationsResponse {
                safe_reason: reason.into(),
                ..Default::default()
            },
        };
        Ok(context.response(response))
    }
    async fn list_github_repositories(
        &self,
        request: Request<v1::ListGithubRepositoriesRequest>,
    ) -> Result<Response<v1::ListGithubRepositoriesResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let page = page(value.page)?;
        identity(value.installation_id)?;
        let source = self
            .store
            .repository_source(&value.connection_id, value.expected_connection_revision)
            .await
            .map_err(|e| context.store(&e))?;
        let result = match github::Github::new(&source) {
            Ok(client) => tokio::time::timeout(
                Duration::from_secs(20),
                client.repositories(value.installation_id, page),
            )
            .await
            .unwrap_or(Err("github_unavailable")),
            Err(reason) => Err(reason),
        };
        self.store
            .repository_source(&value.connection_id, value.expected_connection_revision)
            .await
            .map_err(|e| context.store(&e))?;
        let response = match result {
            Ok(items) => v1::ListGithubRepositoriesResponse {
                next_page: if items.len() == 100 && page < 1000 {
                    page + 1
                } else {
                    0
                },
                repositories: items
                    .into_iter()
                    .map(|v| v1::GithubRepository {
                        provider_repository_id: v.provider_repository_id,
                        full_name: v.full_name,
                        default_branch: v.default_branch,
                        private: v.private,
                    })
                    .collect(),
                safe_reason: String::new(),
            },
            Err(reason) => v1::ListGithubRepositoriesResponse {
                safe_reason: reason.into(),
                ..Default::default()
            },
        };
        Ok(context.response(response))
    }
    async fn connect_repository(
        &self,
        request: Request<v1::ConnectRepositoryRequest>,
    ) -> Result<Response<v1::ConnectRepositoryResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let mut op = self
            .store
            .begin_repository_connect(Connect {
                key: &value.idempotency_key,
                connection_id: &value.connection_id,
                expected_revision: value.expected_connection_revision,
                installation_id: value.installation_id,
                provider_repository_id: value.provider_repository_id,
            })
            .await
            .map_err(|e| context.store(&e))?;
        if op.state == "accepted" {
            let observation = match self
                .store
                .repository_source(&op.connection_id, op.connection_revision)
                .await
            {
                Ok(source) => match github::Github::new(&source) {
                    Ok(client) => tokio::time::timeout(
                        Duration::from_secs(20),
                        client.repository(op.installation_id, op.provider_repository_id),
                    )
                    .await
                    .unwrap_or(Err("github_unavailable")),
                    Err(reason) => Err(reason),
                },
                Err(StoreError::ConfigurationRevisionConflict) => Err("github_source_changed"),
                Err(error) => return Err(context.store(&error)),
            };
            op = self
                .store
                .finish_repository_connect(&value.idempotency_key, observation)
                .await
                .map_err(|e| context.store(&e))?;
        }
        tracing::info!(operation_id=%op.id,connection_id=%op.connection_id,state=%op.state,reason=%op.safe_reason,"Repository source access observed");
        Ok(context.response(v1::ConnectRepositoryResponse {
            operation: Some(operation(op)),
        }))
    }
    async fn get_repository_operation(
        &self,
        request: Request<v1::GetRepositoryOperationRequest>,
    ) -> Result<Response<v1::GetRepositoryOperationResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let op = self
            .store
            .get_repository_operation(&value.idempotency_key)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::GetRepositoryOperationResponse {
            operation: Some(operation(op)),
        }))
    }
    async fn list_repository_operations(
        &self,
        request: Request<v1::ListRepositoryOperationsRequest>,
    ) -> Result<Response<v1::ListRepositoryOperationsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let (items, next) = self
            .store
            .list_repository_operations(&value.after_id, value.limit)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::ListRepositoryOperationsResponse {
            operations: items.into_iter().map(operation).collect(),
            next_after_id: next,
        }))
    }
    async fn list_repositories(
        &self,
        request: Request<v1::ListRepositoriesRequest>,
    ) -> Result<Response<v1::ListRepositoriesResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let (items, next) = self
            .store
            .list_repositories(None, &value.after_id, value.limit)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::ListRepositoriesResponse {
            repositories: items.into_iter().map(repository).collect(),
            next_after_id: next,
        }))
    }
    async fn associate_project_repository(
        &self,
        request: Request<v1::AssociateProjectRepositoryRequest>,
    ) -> Result<Response<v1::AssociateProjectRepositoryResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let (item, command) = self
            .store
            .associate_repository(
                &value.idempotency_key,
                &value.workspace_id,
                &value.project_id,
                &value.repository_id,
            )
            .await
            .map_err(|e| context.store(&e))?;
        tracing::info!(workspace_id=%value.workspace_id,project_id=%value.project_id,repository_id=%item.id,command_id=%command,"Project repository associated");
        Ok(context.response(v1::AssociateProjectRepositoryResponse {
            repository: Some(repository(item)),
            command_id: command,
        }))
    }
    async fn list_project_repositories(
        &self,
        request: Request<v1::ListProjectRepositoriesRequest>,
    ) -> Result<Response<v1::ListProjectRepositoriesResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let (items, next) = self
            .store
            .list_repositories(
                Some((&value.workspace_id, &value.project_id)),
                &value.after_id,
                value.limit,
            )
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::ListProjectRepositoriesResponse {
            repositories: items.into_iter().map(repository).collect(),
            next_after_id: next,
        }))
    }
}
