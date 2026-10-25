use super::{RequestContext, WorkflowStore, plans, v1};
use ayni_orchestrator_events::task_publication::PublicationRun;
use ayni_orchestrator_observability::workflow::{Workflow, finish, identity_context};
use ayni_orchestrator_workflow_postgres::store::publications::PublicationReceipt;
use tonic::{Request, Response, Status};
use tracing::Instrument as _;
pub(super) async fn start(
    store: &WorkflowStore,
    request: Request<v1::StartTaskPublicationRequest>,
) -> Result<Response<v1::StartTaskPublicationResponse>, Status> {
    let context = RequestContext::new(&request);
    let v = request.into_inner();
    let span = Workflow {
        workspace_id: v.workspace_id.clone(),
        project_id: v.project_id.clone(),
        conversation_id: v.conversation_id.clone(),
        task_id: v.task_id.clone(),
        attempt_id: v.attempt_id.clone(),
        ..Workflow::default()
    }
    .span_with_context(
        "task.api.publish",
        identity_context(&v.attempt_id).as_ref(),
        false,
    );
    let result = store
        .start_task_publication(
            plans::scope(&v.workspace_id, &v.project_id, &v.conversation_id),
            &v.task_id,
            &v.attempt_id,
            &v.idempotency_key,
            &v.expected_patch_digest,
            &v.expected_quality_digest,
        )
        .instrument(span.clone())
        .await;
    finish(&span, result.is_ok());
    let result = result.map_err(|e| context.store(&e))?;
    Ok(context.response(v1::StartTaskPublicationResponse {
        receipt: Some(receipt(result)),
    }))
}
pub(super) async fn cancel(
    store: &WorkflowStore,
    request: Request<v1::CancelTaskPublicationRequest>,
) -> Result<Response<v1::CancelTaskPublicationResponse>, Status> {
    let context = RequestContext::new(&request);
    let v = request.into_inner();
    let result = store
        .cancel_task_publication(
            plans::scope(&v.workspace_id, &v.project_id, &v.conversation_id),
            &v.task_id,
            &v.attempt_id,
            &v.idempotency_key,
        )
        .await
        .map_err(|e| context.store(&e))?;
    Ok(context.response(v1::CancelTaskPublicationResponse {
        receipt: Some(receipt(result)),
    }))
}
fn receipt(r: PublicationReceipt) -> v1::TaskPublicationCommandReceipt {
    v1::TaskPublicationCommandReceipt {
        command_id: r.command_id,
        publication_id: r.publication_id,
        attempt_id: r.attempt_id,
        conversation_revision: r.conversation_revision,
        resume_cursor: r.resume_cursor,
    }
}
pub(super) fn projection(run: PublicationRun) -> v1::TaskPublication {
    let outcome = run.outcome.unwrap_or_default();
    v1::TaskPublication {
        publication_id: run.command.publication_id,
        attempt_id: run.command.attempt_id,
        state: run.state,
        generation: run.generation,
        branch: run.command.branch,
        base_branch: run.command.base_branch,
        commit_sha: run.commit_sha,
        tree_sha: run.tree_sha,
        pr_number: outcome.pr_number,
        pr_url: outcome.pr_url,
        pr_state: outcome.pr_state,
        safe_reason: outcome.safe_reason,
        quality_digest: run.command.quality_digest,
        patch_digest: run.command.patch_digest,
        repository: run.command.repository,
    }
}
