//! Public signer configuration; private material stays in administrator-owned Secrets.
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
