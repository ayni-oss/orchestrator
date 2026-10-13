use super::*;
#[tonic::async_trait]
impl v1::connection_service_server::ConnectionService for GrpcApi {
    type WatchConnectionsStream = super::BoxGrpcStream<v1::WatchConnectionsResponse>;
    async fn watch_connections(
        &self,
        request: Request<v1::WatchConnectionsRequest>,
    ) -> Result<Response<Self::WatchConnectionsStream>, Status> {
        let context = self.context(&request, "WatchConnections")?;
        if context.client_id.is_some() {
            return Err(Status::permission_denied("trusted_operator_required"));
        }
        Ok(
            context.response(Box::pin(super::resource_watch::connections(
                self.state.clone(),
                self.shutdown.clone(),
                request.into_inner(),
                context.request_id.clone(),
            ))),
        )
    }

    async fn list_connections(
        &self,
        _: Request<v1::ListConnectionsRequest>,
    ) -> Result<Response<v1::ListConnectionsResponse>, Status> {
        Ok(Response::new(v1::ListConnectionsResponse {
            connections: self.state.connections().await?,
        }))
    }
    async fn get_connection(
        &self,
        request: Request<v1::GetConnectionRequest>,
    ) -> Result<Response<v1::GetConnectionResponse>, Status> {
        Ok(Response::new(v1::GetConnectionResponse {
            connection: Some(self.state.connection(request.into_inner().id).await?),
        }))
    }
    async fn create_connection(
        &self,
        request: Request<v1::CreateConnectionRequest>,
    ) -> Result<Response<v1::CreateConnectionResponse>, Status> {
        Ok(Response::new(v1::CreateConnectionResponse {
            connection: Some(self.state.create_connection(request.into_inner()).await?),
        }))
    }
    async fn replace_connection_credentials(
        &self,
        request: Request<v1::ReplaceConnectionCredentialsRequest>,
    ) -> Result<Response<v1::ReplaceConnectionCredentialsResponse>, Status> {
        Ok(Response::new(v1::ReplaceConnectionCredentialsResponse {
            connection: Some(self.state.replace_connection(request.into_inner()).await?),
        }))
    }
    async fn test_connection(
        &self,
        request: Request<v1::TestConnectionRequest>,
    ) -> Result<Response<v1::TestConnectionResponse>, Status> {
        let request = request.into_inner();
        Ok(Response::new(v1::TestConnectionResponse {
            connection: Some(
                self.state
                    .test_connection(
                        request.id,
                        request.expected_resource_version,
                        request.idempotency_key,
                    )
                    .await?,
            ),
        }))
    }
    async fn set_connection_enabled(
        &self,
        request: Request<v1::SetConnectionEnabledRequest>,
    ) -> Result<Response<v1::SetConnectionEnabledResponse>, Status> {
        let request = request.into_inner();
        Ok(Response::new(v1::SetConnectionEnabledResponse {
            connection: Some(
                self.state
                    .enable_connection(
                        request.id,
                        request.expected_resource_version,
                        request.enabled,
                        request.idempotency_key,
                    )
                    .await?,
            ),
        }))
    }
    async fn delete_connection(
        &self,
        request: Request<v1::DeleteConnectionRequest>,
    ) -> Result<Response<v1::DeleteConnectionResponse>, Status> {
        let request = request.into_inner();
        self.state
            .remove_connection(
                request.id,
                request.expected_resource_version,
                request.idempotency_key,
            )
            .await?;
        Ok(Response::new(v1::DeleteConnectionResponse {}))
    }
    async fn complete_github_manifest(
        &self,
        request: Request<v1::CompleteGithubManifestRequest>,
    ) -> Result<Response<v1::CompleteGithubManifestResponse>, Status> {
        Ok(Response::new(v1::CompleteGithubManifestResponse {
            connection: Some(self.state.complete_manifest(request.into_inner()).await?),
        }))
    }
}
