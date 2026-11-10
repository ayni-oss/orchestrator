//! Public signer configuration; private material stays in administrator-owned Secrets.
pub(super) mod onboarding;
mod operations;
mod profile;
mod provisioning;
use super::{RequestContext, WorkflowGrpc, v1};
use ayni_orchestrator_workflow_postgres::store::runtime_signers::{SaveSigner, Signer};
use std::collections::BTreeMap;
use tonic::{Request, Response, Status};
fn signer(value: Signer) -> v1::RuntimeSigner {
    v1::RuntimeSigner {
        id: value.id,
        display_name: value.display_name,
        key_id: value.key_id,
        public_key: value.public_key,
        secret_name: value.secret_name,
        secret_uid: value.secret_uid,
        active_revision: value.active_revision,
        revision: value.revision,
        enabled: value.enabled,
    }
}
#[tonic::async_trait]
impl v1::runtime_preparation_service_server::RuntimePreparationService for WorkflowGrpc {
    async fn start_repository_runtime(
        &self,
        request: Request<v1::StartRepositoryRuntimeRequest>,
    ) -> Result<Response<v1::StartRepositoryRuntimeResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let run = self
            .store
            .start_repository_runtime(&value.idempotency_key, onboarding::settings(value.clone()))
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::StartRepositoryRuntimeResponse {
            run: Some(onboarding::public(run)),
        }))
    }
    async fn get_repository_runtime(
        &self,
        request: Request<v1::GetRepositoryRuntimeRequest>,
    ) -> Result<Response<v1::GetRepositoryRuntimeResponse>, Status> {
        let context = RequestContext::new(&request);
        let run = self
            .store
            .latest_repository_runtime(&request.into_inner().repository_id)
            .await
            .map_err(|e| context.store(&e))?;
        let profile = profile::RuntimeConfiguration::from_environment().ok();
        Ok(context.response(v1::GetRepositoryRuntimeResponse {
            run: run.map(onboarding::public),
            preparation_enabled: profile.is_some(),
            development_only: profile.is_some_and(|value| {
                value.profile_id
                    == ayni_orchestrator_events::runtime_preparation::DEVELOPMENT_PROFILE_ID
            }),
        }))
    }
    async fn get_repository_runtime_receipt(
        &self,
        request: Request<v1::GetRepositoryRuntimeReceiptRequest>,
    ) -> Result<Response<v1::GetRepositoryRuntimeReceiptResponse>, Status> {
        let context = RequestContext::new(&request);
        let run = self
            .store
            .repository_runtime_receipt(&request.into_inner().idempotency_key)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::GetRepositoryRuntimeReceiptResponse {
            run: Some(onboarding::public(run)),
        }))
    }

    async fn provision_runtime_signer(
        &self,
        request: Request<v1::ProvisionRuntimeSignerRequest>,
    ) -> Result<Response<v1::ProvisionRuntimeSignerResponse>, Status> {
        let context = RequestContext::new(&request);
        let result = provisioning::provision(&self.store, &context, request.into_inner()).await?;
        Ok(context.response(result))
    }
    async fn recover_runtime_signer_provisioning(
        &self,
        request: Request<v1::RecoverRuntimeSignerProvisioningRequest>,
    ) -> Result<Response<v1::RecoverRuntimeSignerProvisioningResponse>, Status> {
        let context = RequestContext::new(&request);
        let result =
            provisioning::recover(&self.store, &context, &request.into_inner().idempotency_key)
                .await?;
        Ok(
            context.response(v1::RecoverRuntimeSignerProvisioningResponse {
                operation: result.operation,
                signer: result.signer,
            }),
        )
    }
    async fn start_runtime_preparation(
        &self,
        request: Request<v1::StartRuntimePreparationRequest>,
    ) -> Result<Response<v1::StartRuntimePreparationResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let operation = operations::start(self, &context, &value).await?;
        Ok(context.response(v1::StartRuntimePreparationResponse {
            operation: Some(operations::public(operation)),
        }))
    }
    async fn get_runtime_preparation(
        &self,
        request: Request<v1::GetRuntimePreparationRequest>,
    ) -> Result<Response<v1::GetRuntimePreparationResponse>, Status> {
        let context = RequestContext::new(&request);
        let operation = self
            .store
            .get_runtime_preparation(&request.into_inner().id)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::GetRuntimePreparationResponse {
            operation: Some(operations::public(operation)),
        }))
    }
    async fn list_runtime_preparations(
        &self,
        request: Request<v1::ListRuntimePreparationsRequest>,
    ) -> Result<Response<v1::ListRuntimePreparationsResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let (items, next) = self
            .store
            .list_runtime_preparations(&value.repository_id, &value.after_id, value.limit)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::ListRuntimePreparationsResponse {
            operations: items.into_iter().map(operations::public).collect(),
            next_after_id: next,
        }))
    }
    async fn get_runtime_preparation_receipt(
        &self,
        request: Request<v1::GetRuntimePreparationReceiptRequest>,
    ) -> Result<Response<v1::GetRuntimePreparationReceiptResponse>, Status> {
        let context = RequestContext::new(&request);
        let operation = self
            .store
            .get_runtime_preparation_receipt(&request.into_inner().idempotency_key)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::GetRuntimePreparationReceiptResponse {
            operation: Some(operations::public(operation)),
        }))
    }
    async fn cancel_runtime_preparation(
        &self,
        request: Request<v1::CancelRuntimePreparationRequest>,
    ) -> Result<Response<v1::CancelRuntimePreparationResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let operation = self
            .store
            .cancel_runtime_preparation(
                &value.idempotency_key,
                &value.id,
                value.expected_generation,
            )
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::CancelRuntimePreparationResponse {
            operation: Some(operations::public(operation)),
        }))
    }
    async fn retry_runtime_preparation(
        &self,
        request: Request<v1::RetryRuntimePreparationRequest>,
    ) -> Result<Response<v1::RetryRuntimePreparationResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let operation = self
            .store
            .retry_runtime_preparation(&value.idempotency_key, &value.id, value.expected_generation)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::RetryRuntimePreparationResponse {
            operation: Some(operations::public(operation)),
        }))
    }
    async fn save_runtime_signer(
        &self,
        request: Request<v1::SaveRuntimeSignerRequest>,
    ) -> Result<Response<v1::SaveRuntimeSignerResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        ayni_core::EnvironmentCertificateTrustPolicy::new(BTreeMap::from([(
            value.key_id.clone(),
            value.public_key.clone(),
        )]))
        .map_err(|_| Status::invalid_argument("public signer identity is invalid"))?;
        let saved = self
            .store
            .save_runtime_signer(SaveSigner {
                key: &value.idempotency_key,
                id: &value.id,
                expected_revision: value.expected_revision,
                display_name: &value.display_name,
                key_id: &value.key_id,
                public_key: &value.public_key,
                secret_name: &value.secret_name,
                secret_uid: &value.secret_uid,
            })
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::SaveRuntimeSignerResponse {
            signer: Some(signer(saved)),
        }))
    }
    async fn set_runtime_signer_enabled(
        &self,
        request: Request<v1::SetRuntimeSignerEnabledRequest>,
    ) -> Result<Response<v1::SetRuntimeSignerEnabledResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let saved = self
            .store
            .set_runtime_signer_enabled(
                &value.idempotency_key,
                &value.id,
                value.expected_revision,
                value.enabled,
            )
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::SetRuntimeSignerEnabledResponse {
            signer: Some(signer(saved)),
        }))
    }
    async fn get_runtime_signer(
        &self,
        request: Request<v1::GetRuntimeSignerRequest>,
    ) -> Result<Response<v1::GetRuntimeSignerResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = self
            .store
            .get_runtime_signer(&request.into_inner().id)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::GetRuntimeSignerResponse {
            signer: Some(signer(value)),
        }))
    }
    async fn delete_runtime_signer(
        &self,
        request: Request<v1::DeleteRuntimeSignerRequest>,
    ) -> Result<Response<v1::DeleteRuntimeSignerResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let removed = self
            .store
            .delete_runtime_signer(&value.idempotency_key, &value.id, value.expected_revision)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::DeleteRuntimeSignerResponse {
            signer: Some(signer(removed)),
        }))
    }
    async fn list_runtime_signers(
        &self,
        request: Request<v1::ListRuntimeSignersRequest>,
    ) -> Result<Response<v1::ListRuntimeSignersResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let (items, next) = self
            .store
            .list_runtime_signers(&value.after_id, value.limit)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::ListRuntimeSignersResponse {
            signers: items.into_iter().map(signer).collect(),
            next_after_id: next,
        }))
    }
    async fn get_runtime_signer_receipt(
        &self,
        request: Request<v1::GetRuntimeSignerReceiptRequest>,
    ) -> Result<Response<v1::GetRuntimeSignerReceiptResponse>, Status> {
        let context = RequestContext::new(&request);
        let value = request.into_inner();
        let saved = self
            .store
            .get_runtime_signer_receipt(&value.idempotency_key)
            .await
            .map_err(|e| context.store(&e))?;
        Ok(context.response(v1::GetRuntimeSignerReceiptResponse {
            signer: Some(signer(saved)),
        }))
    }
}
