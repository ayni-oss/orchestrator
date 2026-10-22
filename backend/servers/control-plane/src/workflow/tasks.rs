use super::{RequestContext, WorkflowStore, plans, v1};
use ayni_orchestrator_workflow_postgres::store::task_runs::RunReceipt;
use tonic::{Request, Response, Status};
pub(super) async fn start(
    store: &WorkflowStore,
    request: Request<v1::StartTaskExecutionRequest>,
) -> Result<Response<v1::StartTaskExecutionResponse>, Status> {
    let context = RequestContext::new(&request);
    let v = request.into_inner();
    let result = store
        .start_task_execution(
            plans::scope(&v.workspace_id, &v.project_id, &v.conversation_id),
            &v.task_id,
            &v.attempt_id,
            &v.idempotency_key,
            &v.source_pack,
        )
        .await
        .map_err(|e| context.store(&e))?;
    Ok(context.response(v1::StartTaskExecutionResponse {
        receipt: Some(receipt(result)),
    }))
}
pub(super) async fn cancel(
    store: &WorkflowStore,
    request: Request<v1::CancelTaskExecutionRequest>,
) -> Result<Response<v1::CancelTaskExecutionResponse>, Status> {
    let context = RequestContext::new(&request);
    let v = request.into_inner();
    let result = store
        .cancel_task_execution(
            plans::scope(&v.workspace_id, &v.project_id, &v.conversation_id),
            &v.task_id,
            &v.attempt_id,
            &v.idempotency_key,
        )
        .await
        .map_err(|e| context.store(&e))?;
    Ok(context.response(v1::CancelTaskExecutionResponse {
        receipt: Some(receipt(result)),
    }))
}
pub(super) async fn get(
    store: &WorkflowStore,
    request: Request<v1::GetTaskAttemptRequest>,
) -> Result<Response<v1::GetTaskAttemptResponse>, Status> {
    let context = RequestContext::new(&request);
    let v = request.into_inner();
    let r = store
        .get_task_run(
            plans::scope(&v.workspace_id, &v.project_id, &v.conversation_id),
            &v.task_id,
            &v.attempt_id,
        )
        .await
        .map_err(|e| context.store(&e))?;
    let outcome = r.outcome.unwrap_or_default();
    let hash = ayni_orchestrator_events::task_execution::digest(outcome.patch.as_bytes());
    Ok(context.response(v1::GetTaskAttemptResponse {
        attempt: Some(v1::TaskAttempt {
            attempt_id: r.command.attempt_id,
            task_id: r.command.task_id,
            state: r.state,
            generation: r.generation,
            pod_uid: r.pod_uid,
            provider_state: r.provider_state,
            patch: outcome.patch,
            safe_reason: outcome.safe_reason,
            patch_digest: hash,
            source_commit: r.command.source_commit,
            runtime_image: r.command.runtime_image,
            configuration_revision_id: r.command.configuration_revision_id,
        }),
    }))
}
fn receipt(r: RunReceipt) -> v1::TaskExecutionCommandReceipt {
    v1::TaskExecutionCommandReceipt {
        command_id: r.command_id,
        attempt_id: r.attempt_id,
        conversation_revision: r.conversation_revision,
        resume_cursor: r.resume_cursor,
    }
}
