//! Durable public intent precedes every external private upload.
mod kubernetes;
mod material;
use super::{RequestContext, signer, v1};
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore,
    store::runtime_signer_provisioning::{BeginSignerProvisioning, SignerProvisioning},
};
use kubernetes::Kubernetes;
use material::Material;
use tonic::Status;
use zeroize::Zeroizing;

pub(super) async fn provision(
    store: &WorkflowStore,
    context: &RequestContext,
    mut request: v1::ProvisionRuntimeSignerRequest,
) -> Result<v1::ProvisionRuntimeSignerResponse, Status> {
    let seed = Zeroizing::new(std::mem::take(&mut request.signing_seed));
    let material = match request.mode.as_str() {
        "generate" if seed.is_empty() => None,
        "import" => Some(Material::imported(&seed).map_err(Status::invalid_argument)?),
        _ => {
            return Err(Status::invalid_argument(
                "signer provisioning mode or upload invalid",
            ));
        }
    };
    let namespace = match store
        .get_runtime_signer_provisioning(&request.idempotency_key)
        .await
    {
        Ok(existing) => existing.namespace,
        Err(StoreError::NotFound) => {
            kubernetes::target_namespace().map_err(Status::failed_precondition)?
        }
        Err(error) => return Err(context.store(&error)),
    };
    let op = store
        .begin_runtime_signer_provisioning(BeginSignerProvisioning {
            key: &request.idempotency_key,
            namespace: &namespace,
            display_name: &request.display_name,
            key_id: &request.key_id,
            mode: &request.mode,
            import_public_key: material
                .as_ref()
                .map_or("", |value| value.public_key.as_str()),
        })
        .await
        .map_err(|error| context.store(&error))?;
    settle(store, context, op, material).await.map(public)
}
pub(super) async fn recover(
    store: &WorkflowStore,
    context: &RequestContext,
    key: &str,
) -> Result<v1::ProvisionRuntimeSignerResponse, Status> {
    let op = store
        .get_runtime_signer_provisioning(key)
        .await
        .map_err(|error| context.store(&error))?;
    settle(store, context, op, None).await.map(public)
}
async fn settle(
    store: &WorkflowStore,
    context: &RequestContext,
    op: SignerProvisioning,
    material: Option<Material>,
) -> Result<SignerProvisioning, Status> {
    if op.signer.is_some() {
        return Ok(op);
    }
    let observed = tokio::time::timeout(std::time::Duration::from_secs(8), execute(&op, material))
        .await
        .unwrap_or(Err("signer_provisioning_unavailable"));
    match observed {
        Ok((uid, public_key)) => {
            store
                .complete_runtime_signer_provisioning(&op.idempotency_key, &uid, &public_key)
                .await
        }
        Err(reason) => {
            store
                .note_runtime_signer_provisioning_failure(&op.idempotency_key, reason)
                .await
        }
    }
    .map_err(|error| context.store(&error))
}
async fn execute(
    op: &SignerProvisioning,
    material: Option<Material>,
) -> Result<(String, String), &'static str> {
    let client = Kubernetes::configured(&op.namespace)?;
    execute_with(&client, op, material).await
}
async fn execute_with(
    client: &Kubernetes,
    op: &SignerProvisioning,
    mut material: Option<Material>,
) -> Result<(String, String), &'static str> {
    if let Some(observed) = client.observed(op).await? {
        return Ok(observed);
    }
    if material.is_none() && op.mode == "generate" {
        material = Some(Material::generated()?);
    }
    let material = material.ok_or("signer_provisioning_inputs_required")?;
    let created = client.create(op, &material).await;
    // A lost POST acknowledgement can still have created the reserved object.
    // Observe once regardless of its transport outcome; never change the name.
    client
        .observed(op)
        .await?
        .ok_or_else(|| created.err().unwrap_or("signer_provisioning_unavailable"))
}
fn public(op: SignerProvisioning) -> v1::ProvisionRuntimeSignerResponse {
    let signer_id = op
        .signer
        .as_ref()
        .map_or_else(String::new, |value| value.id.clone());
    v1::ProvisionRuntimeSignerResponse {
        operation: Some(v1::RuntimeSignerProvisioningOperation {
            id: op.id,
            idempotency_key: op.idempotency_key,
            display_name: op.display_name,
            key_id: op.key_id,
            mode: op.mode,
            state: op.state,
            safe_reason: op.safe_reason,
            public_key: op.public_key,
            secret_name: op.secret_name,
            secret_uid: op.secret_uid,
            signer_id,
            accepted_at_unix_ms: op.accepted_at_unix_ms,
            updated_at_unix_ms: op.updated_at_unix_ms,
        }),
        signer: op.signer.map(signer),
    }
}
