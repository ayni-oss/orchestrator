//! Trusted local-operator connection commands and safe projections.
mod github;
use super::{RequestContext, WorkflowGrpc, v1};
use ayni_orchestrator_workflow_postgres::store::connections::{self, SaveGithub};
use tonic::{Request, Response, Status};

fn connection(c: connections::Connection) -> v1::Connection {
    v1::Connection {
        id: c.id,
        display_name: c.display_name,
        provider: c.provider,
        capability: c.capability,
        revision: c.revision,
        enabled: c.enabled,
        app_id: c.app_id,
        active_credential_revision: c.active_credential_revision,
        candidate_credential_revision: c.candidate_credential_revision,
        status: c.status,
        safe_reason: c.safe_reason,
    }
}
fn operation(o: connections::Operation) -> v1::ConnectionOperation {
    v1::ConnectionOperation {
        id: o.id,
        connection_id: o.connection_id,
        kind: o.kind,
        state: o.state,
        safe_reason: o.safe_reason,
        connection_revision: o.connection_revision,
        connection: Some(connection(o.connection)),
    }
}
#[tonic::async_trait]
impl v1::connection_service_server::ConnectionService for WorkflowGrpc {
    async fn save_github_connection(
        &self,
        request: Request<v1::SaveGithubConnectionRequest>,
    ) -> Result<Response<v1::SaveGithubConnectionResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let result = self
            .store
            .save_github_connection(SaveGithub {
                key: &value.idempotency_key,
                id: &value.connection_id,
                expected: value.expected_revision,
                name: &value.display_name,
                app_id: &value.app_id,
                private_key_pem: &value.private_key_pem,
            })
            .await
            .map_err(|e| context.store(&e))?;
        tracing::info!(operation_id = %result.id, connection_id = %result.connection_id, state = %result.state, "GitHub connection candidate retained");
        Ok(context.response(v1::SaveGithubConnectionResponse {
            operation: Some(operation(result)),
        }))
    }
    async fn set_connection_enabled(
        &self,
        request: Request<v1::SetConnectionEnabledRequest>,
    ) -> Result<Response<v1::SetConnectionEnabledResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let result = self
            .store
            .set_connection_enabled(
                &value.idempotency_key,
                &value.connection_id,
                value.expected_revision,
                value.enabled,
            )
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::SetConnectionEnabledResponse {
            operation: Some(operation(result)),
        }))
    }
    async fn validate_github_connection(
        &self,
        request: Request<v1::ValidateGithubConnectionRequest>,
    ) -> Result<Response<v1::ValidateGithubConnectionResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let candidate = self
            .store
            .github_candidate(&value.operation_id)
            .await
            .map_err(|e| context.store(&e))?;
        let result = if candidate.operation.state == "accepted" {
            let reason = github::validate(&candidate.app_id, &candidate.private_key_pem)
                .await
                .err()
                .unwrap_or("");
            self.store
                .finish_github_validation(&value.operation_id, reason)
                .await
                .map_err(|e| context.store(&e))?
        } else {
            candidate.operation
        };
        tracing::info!(operation_id = %result.id, connection_id = %result.connection_id, state = %result.state, reason = %result.safe_reason, "GitHub connection validation observed");
        Ok(context.response(v1::ValidateGithubConnectionResponse {
            operation: Some(operation(result)),
        }))
    }
    async fn get_connection_operation(
        &self,
        request: Request<v1::GetConnectionOperationRequest>,
    ) -> Result<Response<v1::GetConnectionOperationResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let result = self
            .store
            .get_connection_operation(&value.idempotency_key)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::GetConnectionOperationResponse {
            operation: Some(operation(result)),
        }))
    }
    async fn list_connections(
        &self,
        request: Request<v1::ListConnectionsRequest>,
    ) -> Result<Response<v1::ListConnectionsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let result = self
            .store
            .list_connections(&value.after_id, value.limit)
            .await
            .map_err(|e| context.store(&e))?;
        let next = if result.len()
            == if value.limit == 0 {
                50
            } else {
                value.limit as usize
            } {
            result.last().map_or_else(String::new, |c| c.id.clone())
        } else {
            String::new()
        };
        Ok(context.response(v1::ListConnectionsResponse {
            connections: result.into_iter().map(connection).collect(),
            next_after_id: next,
        }))
    }
    async fn list_connection_operations(
        &self,
        request: Request<v1::ListConnectionOperationsRequest>,
    ) -> Result<Response<v1::ListConnectionOperationsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let result = self
            .store
            .list_connection_operations(&value.connection_id, &value.after_id, value.limit)
            .await
            .map_err(|e| context.store(&e))?;
        let next = if result.len()
            == if value.limit == 0 {
                50
            } else {
                value.limit as usize
            } {
            result.last().map_or_else(String::new, |c| c.id.clone())
        } else {
            String::new()
        };
        Ok(context.response(v1::ListConnectionOperationsResponse {
            operations: result.into_iter().map(operation).collect(),
            next_after_id: next,
        }))
    }
}
