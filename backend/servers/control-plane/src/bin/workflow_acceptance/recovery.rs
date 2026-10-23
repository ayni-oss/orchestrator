//! Cumulative review-only recovery evidence on actual deployed task attempts.
use super::{Client, Result};
use ayni_orchestrator_api::v1;
use prost::Message as _;
use uuid::Uuid;
fn list(start: &v1::StartTaskExecutionRequest) -> v1::ListTaskRecoveryRequest {
    v1::ListTaskRecoveryRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        task_id: start.task_id.clone(),
        attempt_id: start.attempt_id.clone(),
        limit: 32,
        ..Default::default()
    }
}
pub async fn read(client: &mut Client, start: &v1::StartTaskExecutionRequest) -> Result<Vec<u8>> {
    let mut page = client.list_task_recovery(list(start)).await?.into_inner();
    page.confirmations.clear();
    Ok(page.encode_to_vec())
}
pub async fn record(client: &mut Client, start: &v1::StartTaskExecutionRequest) -> Result<Vec<u8>> {
    let feedback = client
        .list_task_feedback(v1::ListTaskFeedbackRequest {
            workspace_id: start.workspace_id.clone(),
            project_id: start.project_id.clone(),
            conversation_id: start.conversation_id.clone(),
            limit: 32,
            ..Default::default()
        })
        .await?
        .into_inner();
    let Some(f) = feedback.feedback.first() else {
        return read(client, start).await;
    };
    let selected = v1::GetTaskAttemptRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        task_id: start.task_id.clone(),
        attempt_id: start.attempt_id.clone(),
    };
    let before = client
        .get_task_attempt(selected.clone())
        .await?
        .into_inner();
    let request = v1::RecordTaskRecoveryRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        task_id: start.task_id.clone(),
        attempt_id: start.attempt_id.clone(),
        idempotency_key: Uuid::now_v7().to_string(),
        content: Some(v1::record_task_recovery_request::Content::Proposal(
            v1::TaskRecoveryProposal {
                feedback_id: f.id.clone(),
                explanation: "Inspect saved changes and failed checks before continuing.".into(),
                action: if f.safe_reason == "quality_failed" {
                    "repair_quality"
                } else {
                    "continue"
                }
                .into(),
                additional_seconds: 300,
                additional_tokens: 4096,
                ..Default::default()
            },
        )),
    };
    let result = client
        .record_task_recovery(request.clone())
        .await?
        .into_inner();
    if client
        .record_task_recovery(request.clone())
        .await?
        .into_inner()
        != result
    {
        return Err("proposal retry changed".into());
    }
    let proposal = result.record.ok_or("proposal missing")?;
    let Some(v1::task_recovery_record::Content::Proposal(p)) = proposal.content else {
        return Err("proposal content missing".into());
    };
    let e = p.evidence.ok_or("evidence missing")?;
    let original = before.attempt.as_ref().ok_or("attempt missing")?;
    if e.patch_digest != original.patch_digest
        || !e.patch_available
        || e.quality_state != original.quality.as_ref().ok_or("quality missing")?.state
        || e.quality_digest.is_empty()
    {
        return Err("saved work differs from accepted evidence".into());
    }
    questions(client, request).await?;
    if client.get_task_attempt(selected).await?.into_inner() != before {
        return Err("review changed task execution authority".into());
    }
    let mut wrong = list(start);
    wrong.conversation_id = Uuid::now_v7().to_string();
    if client.list_task_recovery(wrong).await.is_ok() {
        return Err("foreign recovery read accepted".into());
    }
    let bytes = read(client, start).await?;
    if v1::ListTaskRecoveryResponse::decode(bytes.as_slice())?
        .records
        .len()
        != 3
    {
        return Err("review records missing or duplicated".into());
    }
    Ok(bytes)
}

async fn questions(client: &mut Client, mut request: v1::RecordTaskRecoveryRequest) -> Result {
    request.idempotency_key = Uuid::now_v7().to_string();
    request.content = Some(v1::record_task_recovery_request::Content::Question(
        v1::TaskQuestion {
            text: "Keep the accepted change?".into(),
        },
    ));
    let q = client
        .record_task_recovery(request.clone())
        .await?
        .into_inner()
        .record
        .ok_or("question missing")?;
    request.idempotency_key = Uuid::now_v7().to_string();
    request.content = Some(v1::record_task_recovery_request::Content::Answer(
        v1::TaskAnswer {
            question_id: q.id,
            text: "Keep saved work; inspect the failed checks.".into(),
        },
    ));
    let answer = client
        .record_task_recovery(request.clone())
        .await?
        .into_inner();
    if client
        .record_task_recovery(request.clone())
        .await?
        .into_inner()
        != answer
    {
        return Err("answer retry changed".into());
    }
    request.idempotency_key = Uuid::now_v7().to_string();
    if client.record_task_recovery(request).await.is_ok() {
        return Err("second answer accepted".into());
    }
    Ok(())
}
