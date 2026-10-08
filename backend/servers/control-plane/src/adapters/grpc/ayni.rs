use super::*;
#[tonic::async_trait]
impl v1::ayni_service_server::AyniService for GrpcApi {
    async fn start_builder_installation(
        &self,
        r: Request<v1::StartBuilderInstallationRequest>,
    ) -> Result<Response<v1::StartBuilderInstallationResponse>, Status> {
        Ok(Response::new(v1::StartBuilderInstallationResponse {
            settings: Some(
                self.state
                    .start_builder_installation(r.into_inner())
                    .await?,
            ),
        }))
    }
    async fn get_builder_installation(
        &self,
        _: Request<v1::GetBuilderInstallationRequest>,
    ) -> Result<Response<v1::GetBuilderInstallationResponse>, Status> {
        Ok(Response::new(v1::GetBuilderInstallationResponse {
            installation: self.state.build_settings().await?.installation,
        }))
    }
    async fn cancel_builder_installation(
        &self,
        r: Request<v1::CancelBuilderInstallationRequest>,
    ) -> Result<Response<v1::CancelBuilderInstallationResponse>, Status> {
        Ok(Response::new(v1::CancelBuilderInstallationResponse {
            settings: Some(
                self.state
                    .cancel_builder_installation(r.into_inner())
                    .await?,
            ),
        }))
    }

    async fn list_ayni_releases(
        &self,
        r: Request<v1::ListAyniReleasesRequest>,
    ) -> Result<Response<v1::ListAyniReleasesResponse>, Status> {
        Ok(Response::new(
            crate::coordinators::build_service::releases(r.into_inner().page).await?,
        ))
    }
    async fn get_build_service(
        &self,
        _: Request<v1::GetBuildServiceRequest>,
    ) -> Result<Response<v1::GetBuildServiceResponse>, Status> {
        Ok(Response::new(v1::GetBuildServiceResponse {
            settings: Some(self.state.build_settings().await?),
        }))
    }
    async fn update_build_service(
        &self,
        r: Request<v1::UpdateBuildServiceRequest>,
    ) -> Result<Response<v1::UpdateBuildServiceResponse>, Status> {
        Ok(Response::new(v1::UpdateBuildServiceResponse {
            settings: Some(self.state.update_build_service(r.into_inner()).await?),
        }))
    }
    async fn test_build_service(
        &self,
        r: Request<v1::TestBuildServiceRequest>,
    ) -> Result<Response<v1::TestBuildServiceResponse>, Status> {
        Ok(Response::new(v1::TestBuildServiceResponse {
            settings: Some(self.state.test_build_service(r.into_inner()).await?),
        }))
    }
    async fn cancel_build_service_test(
        &self,
        r: Request<v1::CancelBuildServiceTestRequest>,
    ) -> Result<Response<v1::CancelBuildServiceTestResponse>, Status> {
        Ok(Response::new(v1::CancelBuildServiceTestResponse {
            settings: Some(self.state.cancel_build_service_test(r.into_inner()).await?),
        }))
    }
}
