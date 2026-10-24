use super::{RequestContext, WorkflowStore, plans, v1};
use ayni_orchestrator_workflow_postgres::store::task_recovery::{
    ProposalInput, RecoveryContent, RecoveryInput, RecoveryRecord,
};
use tonic::{Request, Response, Status};

pub(super) async fn record(
    store: &WorkflowStore,
    request: Request<v1::RecordTaskRecoveryRequest>,
) -> Result<Response<v1::RecordTaskRecoveryResponse>, Status> {
    use v1::record_task_recovery_request::Content;
    let context = RequestContext::new(&request);
    let v = request.into_inner();
    let input = match v
        .content
        .ok_or_else(|| Status::invalid_argument("content required"))?
    {
        Content::Proposal(p) => {
            if p.evidence.is_some() || !p.safe_reason.is_empty() {
                return Err(Status::invalid_argument(
                    "proposal evidence is server derived",
                ));
            }
            RecoveryInput::Proposal(ProposalInput {
                feedback_id: p.feedback_id,
                explanation: p.explanation,
                action: p.action,
                additional_seconds: p.additional_seconds,
                additional_tokens: p.additional_tokens,
            })
        }
        Content::Question(q) => RecoveryInput::Question { text: q.text },
        Content::Answer(a) => RecoveryInput::Answer {
            question_id: a.question_id,
            text: a.text,
        },
    };
    let record = store
        .record_task_recovery(
            plans::scope(&v.workspace_id, &v.project_id, &v.conversation_id),
            &v.task_id,
            &v.attempt_id,
            &v.idempotency_key,
            &input,
        )
        .await
        .map_err(|e| context.store(&e))?;
    Ok(context.response(v1::RecordTaskRecoveryResponse {
        record: Some(projection(record)),
    }))
}
pub(super) async fn list(
    store: &WorkflowStore,
    request: Request<v1::ListTaskRecoveryRequest>,
) -> Result<Response<v1::ListTaskRecoveryResponse>, Status> {
    let context = RequestContext::new(&request);
    let v = request.into_inner();
    let page = store
        .list_task_recovery(
            plans::scope(&v.workspace_id, &v.project_id, &v.conversation_id),
            &v.task_id,
            &v.attempt_id,
            &v.after_id,
            v.limit,
        )
        .await
        .map_err(|e| context.store(&e))?;
    Ok(context.response(v1::ListTaskRecoveryResponse {
        records: page.records.into_iter().map(projection).collect(),
        next_after_id: page.next_after_id,
        confirmations: page.confirmations.into_iter().map(confirmation).collect(),
        question_answers: page.question_answers.into_iter().map(projection).collect(),
    }))
}
fn projection(r: RecoveryRecord) -> v1::TaskRecoveryRecord {
    use v1::task_recovery_record::Content;
    let content = match r.content {
        RecoveryContent::Proposal(p) => {
            let e = p.evidence;
            Content::Proposal(v1::TaskRecoveryProposal {
                feedback_id: p.request.feedback_id,
                explanation: p.request.explanation,
                action: p.request.action,
                additional_seconds: p.request.additional_seconds,
                additional_tokens: p.request.additional_tokens,
                safe_reason: p.safe_reason,
                evidence: Some(v1::TaskSavedEvidence {
                    state: e.state,
                    generation: e.generation,
                    provider_uncertain: e.provider_uncertain,
                    source_commit: e.source_commit,
                    runtime_image: e.runtime_image,
                    configuration_revision_id: e.configuration_revision_id,
                    patch_available: e.patch_available,
                    patch_digest: e.patch_digest,
                    outcome_digest: e.outcome_digest,
                    quality_digest: e.quality_digest,
                    quality_state: e.quality_state,
                    limitations: e.limitations,
                }),
            })
        }
        RecoveryContent::Question { text } => Content::Question(v1::TaskQuestion { text }),
        RecoveryContent::Answer { question_id, text } => {
            Content::Answer(v1::TaskAnswer { question_id, text })
        }
    };
    v1::TaskRecoveryRecord {
        id: r.id,
        workspace_id: r.workspace_id,
        project_id: r.project_id,
        conversation_id: r.conversation_id,
        task_id: r.task_id,
        attempt_id: r.attempt_id,
        kind: r.kind,
        content_digest: r.content_digest,
        created_at: Some(super::timestamp(r.created_at)),
        author: r.author,
        content: Some(content),
    }
}

pub(super) async fn confirm(
    store: &WorkflowStore,
    request: Request<v1::ConfirmTaskRecoveryRequest>,
) -> Result<Response<v1::ConfirmTaskRecoveryResponse>, Status> {
    use ayni_orchestrator_workflow_postgres::store::task_recovery::ConfirmRecovery;
    let context = RequestContext::new(&request);
    let v = request.into_inner();
    let result = store
        .confirm_task_recovery(
            plans::scope(&v.workspace_id, &v.project_id, &v.conversation_id),
            &v.task_id,
            &ConfirmRecovery {
                proposal_id: &v.proposal_id,
                proposal_digest: &v.proposal_digest,
                key: &v.idempotency_key,
                acknowledge_uncertainty: v.acknowledge_provider_uncertainty,
            },
        )
        .await
        .map_err(|e| context.store(&e))?;
    Ok(context.response(v1::ConfirmTaskRecoveryResponse {
        confirmation: Some(confirmation(result)),
    }))
}
fn confirmation(
    r: ayni_orchestrator_workflow_postgres::store::task_recovery::RecoveryConfirmation,
) -> v1::TaskRecoveryConfirmation {
    v1::TaskRecoveryConfirmation {
        command_id: r.command_id,
        proposal_id: r.proposal_id,
        proposal_digest: r.proposal_digest,
        parent_attempt_id: r.parent_attempt_id,
        attempt_id: r.attempt_id,
        conversation_revision: r.conversation_revision,
        resume_cursor: r.resume_cursor,
        acknowledged_provider_uncertainty: r.acknowledged_provider_uncertainty,
    }
}
