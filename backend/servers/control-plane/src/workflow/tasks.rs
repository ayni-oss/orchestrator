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
    let publication = store
        .get_task_publication(
            plans::scope(&v.workspace_id, &v.project_id, &v.conversation_id),
            &v.task_id,
            &v.attempt_id,
        )
        .await
        .map_err(|e| context.store(&e))?;
    let quality_digest = r
        .quality
        .as_ref()
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|_| Status::internal("quality evidence unavailable"))?
        .map(|b| ayni_orchestrator_events::task_execution::digest(&b))
        .unwrap_or_default();
    let outcome = r.outcome.unwrap_or_default();
    let hash = ayni_orchestrator_events::task_execution::digest(outcome.patch.as_bytes());
    Ok(context.response(v1::GetTaskAttemptResponse {
        attempt: Some(v1::TaskAttempt {
            max_seconds: r.command.max_seconds,
            max_tokens: r.command.max_tokens,
            saved_input_patch: r
                .command
                .recovery
                .as_ref()
                .map(|c| c.seed_patch.clone())
                .unwrap_or_default(),
            parent_attempt_id: r
                .command
                .recovery
                .as_ref()
                .map(|c| c.parent_attempt_id.clone())
                .unwrap_or_default(),
            recovery_proposal_id: r
                .command
                .recovery
                .as_ref()
                .map(|c| c.proposal_id.clone())
                .unwrap_or_default(),
            execution_outcome: outcome.state,
            quality_digest,
            publication: publication.map(super::publications::projection),
            quality: r.quality.map(|q| v1::TaskQualityOutcome {
                state: q.state,
                safe_reason: q.safe_reason,
                snapshot_digest: q.snapshot_digest,
                lock_digest: q.lock_digest,
                runtime_image: q.runtime_image,
                runtime_platform: q.runtime_platform,
                runtime_metadata_digest: q.runtime_metadata_digest,
                executor_digest: q.executor_digest,
                exit_code: q.exit_code,
                artifact_digest: q.artifact_digest,
                artifact: q.artifact,
                expected_targets: q.expected_targets,
                completed_targets: q.completed_targets,
                contract_digest: q.contract_digest,
                lock_fingerprint: q.lock_fingerprint,
                source_fingerprint: q.source_fingerprint,
                validation_input_digest: q.validation_input_digest,
            }),
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

pub(super) async fn feedback(
    store: &WorkflowStore,
    request: Request<v1::ListTaskFeedbackRequest>,
) -> Result<Response<v1::ListTaskFeedbackResponse>, Status> {
    let context = RequestContext::new(&request);
    let value = request.into_inner();
    let page = store
        .list_task_feedback(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
            &value.after_id,
            value.limit,
        )
        .await
        .map_err(|e| context.store(&e))?;
    Ok(context.response(v1::ListTaskFeedbackResponse {
        feedback: page
            .feedback
            .into_iter()
            .map(|v| v1::TaskFeedback {
                id: v.id,
                workspace_id: v.workspace_id,
                project_id: v.project_id,
                conversation_id: v.conversation_id,
                task_id: v.task_id,
                attempt_id: v.attempt_id,
                category: v.category,
                state: "open".into(),
                safe_reason: v.safe_reason,
                summary: v.summary,
                created_at: Some(super::timestamp(v.created_at)),
            })
            .collect(),
        next_after_id: page.next_after_id,
    }))
}
