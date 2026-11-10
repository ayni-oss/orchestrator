//! Explicit publication of the two-file signing trust update. Source discovery remains read-only.
mod lock;
mod publisher;

use super::{RequestContext, WorkflowGrpc, v1};
use crate::workflow::repositories::runtime_source_inputs;
use ayni_orchestrator_events::task_publication::base_branch;
use std::time::Duration;
use tonic::Status;

pub(super) fn enabled(repository: &str) -> bool {
    std::env::var("ORCHESTRATOR_SIGNING_UPDATE_REPOSITORY").as_deref() == Ok(repository)
        && std::env::var_os("ORCHESTRATOR_SIGNING_UPDATE_TOKEN_FILE").is_some()
}

pub(super) async fn run(
    service: &WorkflowGrpc,
    context: &RequestContext,
    value: v1::OpenRuntimeSigningPullRequestRequest,
    publish: bool,
) -> Result<Option<v1::RuntimeSigningPullRequest>, Status> {
    let repository = service
        .store
        .inspection_repository(&value.repository_id)
        .await
        .map_err(|error| context.store(&error))?;
    if !enabled(&repository.full_name) {
        return Err(Status::failed_precondition(
            "Configure a signing update publisher for this repository.",
        ));
    }
    let inspected = service
        .store
        .get_runtime_inspection_by_id(&value.inspection_id)
        .await
        .map_err(|error| context.store(&error))?;
    if inspected.repository_id != repository.id
        || inspected.connection_revision != value.expected_connection_revision
        || inspected.state != "completed"
        || inspected.result.as_ref().is_none_or(|result| {
            result.findings.iter().any(|finding| {
                !matches!(
                    finding.code.as_str(),
                    "adapter_discovery_pending" | "runtime_verification_pending"
                )
            })
        })
    {
        return Err(Status::failed_precondition(
            "Check ayni.lock again before publishing the signing key update.",
        ));
    }
    let signer = reviewed_signer(service, context, &value).await?;
    let base = base_branch(&format!("refs/heads/{}", value.base_branch))
        .ok_or_else(|| Status::invalid_argument("The target branch is invalid."))?;
    let source = service
        .store
        .repository_source(
            &repository.connection_id,
            value.expected_connection_revision,
        )
        .await
        .map_err(|error| context.store(&error))?;
    let inputs = tokio::time::timeout(
        Duration::from_secs(30),
        runtime_source_inputs(&source, &repository, &inspected.commit_sha),
    )
    .await
    .map_err(|_| Status::unavailable("Reading the inspected commit timed out."))?
    .map_err(|_| {
        Status::failed_precondition(
            "Could not read the reviewed policy and lock from the inspected commit.",
        )
    })?;
    let (policy, lock) = lock::update(
        &inputs.policy,
        &inputs.lock,
        &signer.key_id,
        &signer.public_key,
    )
    .map_err(Status::failed_precondition)?;
    service
        .store
        .repository_source(
            &repository.connection_id,
            value.expected_connection_revision,
        )
        .await
        .map_err(|error| context.store(&error))?;
    reviewed_signer(service, context, &value).await?;
    let request = publisher::Change {
        repository: &repository.full_name,
        repository_id: repository.provider_repository_id,
        base: &base,
        commit: &inspected.commit_sha,
        tree: &inputs.tree_sha,
        policy: &policy,
        lock: &lock,
    };
    if publish {
        publisher::open(request).await.map(Some)
    } else {
        publisher::lookup(request).await
    }
}

async fn reviewed_signer(
    service: &WorkflowGrpc,
    context: &RequestContext,
    value: &v1::OpenRuntimeSigningPullRequestRequest,
) -> Result<ayni_orchestrator_workflow_postgres::store::runtime_signers::Signer, Status> {
    let signer = service
        .store
        .get_runtime_signer(&value.signer_id)
        .await
        .map_err(|error| context.store(&error))?;
    if !signer.enabled || signer.revision != value.expected_signer_revision {
        return Err(Status::aborted(
            "The selected signing key changed. Review it again.",
        ));
    }
    Ok(signer)
}
