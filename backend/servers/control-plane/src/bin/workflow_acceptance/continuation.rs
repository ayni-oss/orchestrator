//! Real supervisor dispatch and exact retained continuation evidence across owner restarts.
use super::{Client, Result};
use ayni_orchestrator_api::v1;
use prost::Message as _;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;
#[derive(Serialize, Deserialize)]
struct Evidence {
    request: Vec<u8>,
    receipt: Vec<u8>,
    parent_request: Vec<u8>,
    parent: String,
    child: String,
}
pub async fn confirm(
    client: &mut Client,
    start: &v1::StartTaskExecutionRequest,
) -> Result<Vec<u8>> {
    let page = client
        .list_task_recovery(v1::ListTaskRecoveryRequest {
            workspace_id: start.workspace_id.clone(),
            project_id: start.project_id.clone(),
            conversation_id: start.conversation_id.clone(),
            task_id: start.task_id.clone(),
            attempt_id: start.attempt_id.clone(),
            limit: 32,
            ..Default::default()
        })
        .await?
        .into_inner();
    let Some(record) = page.records.iter().find(|r| r.kind == "proposal") else {
        return Ok(Vec::new());
    };
    let parent_request = v1::GetTaskAttemptRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        task_id: start.task_id.clone(),
        attempt_id: start.attempt_id.clone(),
    };
    let parent = client
        .get_task_attempt(parent_request.clone())
        .await?
        .into_inner();
    let request = v1::ConfirmTaskRecoveryRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        task_id: start.task_id.clone(),
        proposal_id: record.id.clone(),
        proposal_digest: record.content_digest.clone(),
        idempotency_key: Uuid::now_v7().to_string(),
        acknowledge_provider_uncertainty: false,
    };
    let mut other = request.clone();
    other.idempotency_key = Uuid::now_v7().to_string();
    let mut second_client = client.clone();
    let (a, b) = tokio::join!(
        client.confirm_task_recovery(request.clone()),
        second_client.confirm_task_recovery(other)
    );
    let receipt = a?.into_inner();
    if receipt != b?.into_inner() {
        return Err("confirmation race did not converge".into());
    }
    let c = receipt.confirmation.as_ref().ok_or("receipt missing")?;
    let mut child_request = parent_request.clone();
    child_request.attempt_id = c.attempt_id.clone();
    let child = wait_for_child(client, child_request).await?;
    let old = parent.attempt.as_ref().ok_or("parent missing")?;
    let new = child.attempt.as_ref().ok_or("child missing")?;
    if new.state != "completed"
        || new.parent_attempt_id != start.attempt_id
        || new.recovery_proposal_id != record.id
        || new.saved_input_patch != old.patch
        || new.source_commit != old.source_commit
        || new.runtime_image != old.runtime_image
        || new.max_seconds != 300
        || new.max_tokens != 4096
        || new.patch.is_empty()
        || new.quality.is_none()
    {
        return Err("continued saved work or frozen inputs missing".into());
    }
    if let Some(v1::task_recovery_record::Content::Proposal(p)) = &record.content
        && p.action == "repair_quality"
        && new.quality.as_ref().is_none_or(|q| q.state != "passed")
    {
        return Err("confirmed repair did not pass managed quality".into());
    }
    let evidence = Evidence {
        request: request.encode_to_vec(),
        receipt: receipt.encode_to_vec(),
        parent_request: parent_request.encode_to_vec(),
        parent: ayni_orchestrator_events::task_execution::digest(
            &super::attempt_evidence::response_bytes(&parent),
        ),
        child: ayni_orchestrator_events::task_execution::digest(
            &super::attempt_evidence::response_bytes(&child),
        ),
    };
    let bytes = serde_json::to_vec(&evidence)?;
    verify(client, &bytes).await?;
    Ok(bytes)
}
pub async fn verify(client: &mut Client, bytes: &[u8]) -> Result {
    if bytes.is_empty() {
        return Ok(());
    }
    let evidence: Evidence = serde_json::from_slice(bytes)?;
    let request = v1::ConfirmTaskRecoveryRequest::decode(evidence.request.as_slice())?;
    if client
        .confirm_task_recovery(request.clone())
        .await?
        .into_inner()
        .encode_to_vec()
        != evidence.receipt
    {
        return Err("confirmation receipt changed after restart".into());
    }
    let receipt = v1::ConfirmTaskRecoveryResponse::decode(evidence.receipt.as_slice())?
        .confirmation
        .ok_or("receipt missing")?;
    let parent_request = v1::GetTaskAttemptRequest::decode(evidence.parent_request.as_slice())?;
    if ayni_orchestrator_events::task_execution::digest(&super::attempt_evidence::response_bytes(
        &client
            .get_task_attempt(parent_request.clone())
            .await?
            .into_inner(),
    )) != evidence.parent
    {
        return Err("recovery overwrote parent evidence".into());
    }
    let child = v1::GetTaskAttemptRequest {
        attempt_id: receipt.attempt_id.clone(),
        ..parent_request
    };
    if ayni_orchestrator_events::task_execution::digest(&super::attempt_evidence::response_bytes(
        &client.get_task_attempt(child).await?.into_inner(),
    )) != evidence.child
    {
        return Err("linked attempt changed after restart".into());
    }
    let current = client
        .get_task_execution(v1::GetTaskExecutionRequest {
            workspace_id: request.workspace_id,
            project_id: request.project_id,
            conversation_id: request.conversation_id,
            task_id: request.task_id,
        })
        .await?
        .into_inner()
        .preparation
        .ok_or("latest attempt missing")?;
    if current.attempt_id != receipt.attempt_id {
        return Err("latest task selection did not follow confirmation".into());
    }
    Ok(())
}

async fn wait_for_child(
    client: &mut Client,
    child_request: v1::GetTaskAttemptRequest,
) -> Result<v1::GetTaskAttemptResponse> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
    let child = loop {
        let child = client
            .get_task_attempt(child_request.clone())
            .await?
            .into_inner();
        if child.attempt.as_ref().is_some_and(|a| {
            [
                "completed",
                "failed",
                "uncertain",
                "cancelled",
                "interrupted",
            ]
            .contains(&a.state.as_str())
        }) {
            break child;
        }
        if tokio::time::Instant::now() > deadline {
            return Err("continued attempt did not finish".into());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    Ok(child)
}
