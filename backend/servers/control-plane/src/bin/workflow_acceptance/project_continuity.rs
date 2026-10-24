//! Trusted in-Pod acceptance inspects exact retained commands without exporting credentials.
use super::{Client, Result};
use ayni_orchestrator_api::v1;
use ayni_orchestrator_control_plane::Configuration;
use ayni_orchestrator_events::workflow::decode_command;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct Evidence {
    request: Vec<u8>,
    receipt: Vec<u8>,
    turn_id: String,
    command: Vec<u8>,
}

pub async fn retain(client: &mut Client, start: &v1::StartTaskExecutionRequest) -> Result<Vec<u8>> {
    use prost::Message as _;
    let conversation = client
        .create_conversation(v1::CreateConversationRequest {
            workspace_id: start.workspace_id.clone(),
            project_id: start.project_id.clone(),
            idempotency_key: Uuid::now_v7().to_string(),
            title: "Project continuity acceptance".into(),
        })
        .await?
        .into_inner()
        .conversation
        .ok_or("continuity conversation")?;
    let request = v1::SendMessageRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: conversation.id,
        idempotency_key: Uuid::now_v7().to_string(),
        content: "Inspect accepted project facts.".into(),
    };
    let receipt = client.send_message(request.clone()).await?.into_inner();
    let turn_id = receipt
        .receipt
        .as_ref()
        .ok_or("continuity receipt")?
        .turn_id
        .clone();
    let command = read(&turn_id).await?;
    let (_, _, payload) = decode_command(&command)?;
    let facts = payload.project_status.ok_or("continuity snapshot")?;
    if facts.project_id != start.project_id
        || facts.total_tasks != Some(1)
        || facts.tasks.len() != 1
        || facts.tasks[0].task_id != start.task_id
        || facts.tasks[0].accepted_coding_state != "completed"
        || facts.status != "registered"
        || payload.context.len() != 1
        || payload.context[0].content != request.content
    {
        return Err("fresh conversation did not receive scoped factual continuity".into());
    }
    let fact = &facts.tasks[0];
    let attempt = client
        .get_task_attempt(v1::GetTaskAttemptRequest {
            workspace_id: start.workspace_id.clone(),
            project_id: start.project_id.clone(),
            conversation_id: start.conversation_id.clone(),
            task_id: start.task_id.clone(),
            attempt_id: fact.attempt_id.clone(),
        })
        .await?
        .into_inner()
        .attempt
        .ok_or("continuity attempt")?;
    #[allow(
        clippy::suspicious_operation_groupings,
        reason = "canonical attempt state maps to factual coding lifecycle while accepted outcome remains separate"
    )]
    if fact.coding_state != attempt.state
        || fact.provider_state != attempt.provider_state
        || fact.accepted_coding_state != attempt.execution_outcome
        || fact.parent_attempt_id != attempt.parent_attempt_id
        || fact.quality_state
            != attempt
                .quality
                .as_ref()
                .map_or("", |quality| quality.state.as_str())
        || fact.quality_artifact_digest
            != attempt
                .quality
                .as_ref()
                .map_or("", |quality| quality.artifact_digest.as_str())
    {
        return Err("continuity facts differ from canonical coding/quality outcomes".into());
    }
    Ok(serde_json::to_vec(&Evidence {
        request: request.encode_to_vec(),
        receipt: receipt.encode_to_vec(),
        turn_id,
        command,
    })?)
}

pub async fn verify(client: &mut Client, bytes: &[u8]) -> Result {
    use prost::Message as _;
    if bytes.is_empty() {
        return Ok(());
    }
    let evidence: Evidence = serde_json::from_slice(bytes)?;
    let request = v1::SendMessageRequest::decode(evidence.request.as_slice())?;
    if client
        .send_message(request)
        .await?
        .into_inner()
        .encode_to_vec()
        != evidence.receipt
        || read(&evidence.turn_id).await? != evidence.command
    {
        return Err("frozen continuity changed across replay or owner restart".into());
    }
    Ok(())
}

async fn read(turn: &str) -> Result<Vec<u8>> {
    // The acceptance binary runs within the trusted control-plane Pod. Existing
    // configuration reads the mounted runtime identity in memory, never stdout.
    let url = Configuration::from_environment()?.runtime_database_url()?;
    let (client, driver) = ayni_orchestrator_workflow_postgres::connect(&url).await?;
    let bytes = client
        .query_one(
            "SELECT exact_command_bytes FROM workflow_dispatch_intents WHERE turn_id=$1",
            &[&turn],
        )
        .await?
        .get(0);
    driver.abort();
    Ok(bytes)
}

pub async fn settle(client: &mut Client, bytes: &[u8]) -> Result {
    use prost::Message as _;
    let evidence: Evidence = serde_json::from_slice(bytes)?;
    let request = v1::SendMessageRequest::decode(evidence.request.as_slice())?;
    wait_for_completed_conversation(client, &request, &evidence.turn_id).await?;
    verify(client, bytes).await
}

async fn wait_for_completed_conversation(
    client: &mut Client,
    request: &v1::SendMessageRequest,
    turn_id: &str,
) -> Result {
    // Completion proves the new session was provisioned and its scripted result
    // accepted before the cumulative profile freezes its retained PVC inventory.
    for _ in 0..240 {
        let snapshot = client
            .get_conversation_snapshot(v1::GetConversationSnapshotRequest {
                workspace_id: request.workspace_id.clone(),
                project_id: request.project_id.clone(),
                conversation_id: request.conversation_id.clone(),
            })
            .await?
            .into_inner()
            .snapshot
            .ok_or("continuity conversation snapshot")?;
        if let Some(terminal) = snapshot
            .recent_terminal_turns
            .iter()
            .find(|turn| turn.id == turn_id)
        {
            if terminal.presentation_state != v1::PresentationState::Completed as i32 {
                return Err("continuity scripted conversation failed".into());
            }
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err("continuity scripted conversation completion deadline".into())
}
