use super::*;
#[tonic::async_trait]
impl v1::certificate_service_server::CertificateService for GrpcApi {
    type WatchCertificatesStream = super::BoxGrpcStream<v1::WatchCertificatesResponse>;
    async fn watch_certificates(
        &self,
        request: Request<v1::WatchCertificatesRequest>,
    ) -> Result<Response<Self::WatchCertificatesStream>, Status> {
        let context = self.context(&request, "WatchCertificates")?;
        if context.client_id.is_some() {
            return Err(Status::permission_denied("trusted_operator_required"));
        }
        Ok(
            context.response(Box::pin(super::resource_watch::certificates(
                self.state.clone(),
                self.shutdown.clone(),
                request.into_inner(),
                context.request_id.clone(),
            ))),
        )
    }

    async fn list_certificates(
        &self,
        request: Request<v1::ListCertificatesRequest>,
    ) -> Result<Response<v1::ListCertificatesResponse>, Status> {
        let _ = request;
        Ok(Response::new(v1::ListCertificatesResponse {
            certificates: self.state.certificates().await?,
        }))
    }
    async fn get_certificate(
        &self,
        request: Request<v1::GetCertificateRequest>,
    ) -> Result<Response<v1::GetCertificateResponse>, Status> {
        Ok(Response::new(v1::GetCertificateResponse {
            certificate: Some(self.state.certificate(request.into_inner().id).await?),
        }))
    }
    async fn create_certificate(
        &self,
        request: Request<v1::CreateCertificateRequest>,
    ) -> Result<Response<v1::CreateCertificateResponse>, Status> {
        Ok(Response::new(v1::CreateCertificateResponse {
            certificate: Some(self.state.create_certificate(request.into_inner()).await?),
        }))
    }
    async fn replace_certificate(
        &self,
        request: Request<v1::ReplaceCertificateRequest>,
    ) -> Result<Response<v1::ReplaceCertificateResponse>, Status> {
        Ok(Response::new(v1::ReplaceCertificateResponse {
            certificate: Some(self.state.replace_certificate(request.into_inner()).await?),
        }))
    }
    async fn rename_certificate(
        &self,
        request: Request<v1::RenameCertificateRequest>,
    ) -> Result<Response<v1::RenameCertificateResponse>, Status> {
        let request = request.into_inner();
        Ok(Response::new(v1::RenameCertificateResponse {
            certificate: Some(
                self.state
                    .mutate_certificate(
                        request.id,
                        request.idempotency_key,
                        request.expected_resource_version,
                        "rename",
                        request.display_name,
                        true,
                    )
                    .await?,
            ),
        }))
    }
    async fn set_certificate_enabled(
        &self,
        request: Request<v1::SetCertificateEnabledRequest>,
    ) -> Result<Response<v1::SetCertificateEnabledResponse>, Status> {
        let request = request.into_inner();
        Ok(Response::new(v1::SetCertificateEnabledResponse {
            certificate: Some(
                self.state
                    .mutate_certificate(
                        request.id,
                        request.idempotency_key,
                        request.expected_resource_version,
                        "enable",
                        String::new(),
                        request.enabled,
                    )
                    .await?,
            ),
        }))
    }
    async fn set_default_certificate(
        &self,
        request: Request<v1::SetDefaultCertificateRequest>,
    ) -> Result<Response<v1::SetDefaultCertificateResponse>, Status> {
        let request = request.into_inner();
        Ok(Response::new(v1::SetDefaultCertificateResponse {
            certificate: Some(
                self.state
                    .mutate_certificate(
                        request.id,
                        request.idempotency_key,
                        request.expected_resource_version,
                        "default",
                        String::new(),
                        true,
                    )
                    .await?,
            ),
        }))
    }
    async fn delete_certificate(
        &self,
        request: Request<v1::DeleteCertificateRequest>,
    ) -> Result<Response<v1::DeleteCertificateResponse>, Status> {
        self.state.delete_certificate(request.into_inner()).await?;
        Ok(Response::new(v1::DeleteCertificateResponse {}))
    }
}
