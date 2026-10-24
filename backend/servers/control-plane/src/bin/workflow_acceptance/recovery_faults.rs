//! Exact recovery receipts and saved work across faults in the continued attempt itself.
use super::{Client, Result, task_runs};
use ayni_orchestrator_api::v1::{self, project_service_client::ProjectServiceClient};
use prost::Message as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{io::Read as _, time::Duration};
use tonic::transport::Channel;
use uuid::Uuid;

#[derive(Deserialize)]
struct Seed {
    request: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
struct Evidence {
    parent_request: Vec<u8>,
    parent: Vec<u8>,
    proposal: Vec<u8>,
    confirmation_request: Vec<u8>,
    confirmation: Vec<u8>,
    worker: String,
    #[serde(default)]
    cancellation_request: Vec<u8>,
    #[serde(default)]
    cancellation: Vec<u8>,
    #[serde(default)]
    terminal: Vec<u8>,
    #[serde(default)]
    feedback: Vec<u8>,
}
const MAX_RECOVERY_EVIDENCE_BYTES: usize = 8 * 1024 * 1024;
fn read<T: DeserializeOwned>() -> Result<T> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(u64::try_from(MAX_RECOVERY_EVIDENCE_BYTES)? + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RECOVERY_EVIDENCE_BYTES {
        return Err("recovery fault evidence bound exceeded".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn emit(e: &Evidence) -> Result {
    println!("{}", serde_json::to_string(e)?);
    Ok(())
}
pub async fn dispatch(
    mode: &str,
    client: &mut Client,
    projects: &mut ProjectServiceClient<Channel>,
) -> Result {
    match mode {
        "--prepare-recovery-fault" => task_runs::prepare_recovery_fault(projects, client).await,
        "--confirm-recovery-fault-stdin" => confirm(client, read()?).await,
        "--verify-recovery-fault-fenced-stdin" => {
            let e: Evidence = read()?;
            receipts(client, &e).await?;
            let child = saved_work(client, &e)
                .await?
                .attempt
                .ok_or("fenced child missing")?;
            if child.state != "cancelling" || child.generation <= 1 {
                return Err("late worker bypassed recovery cancellation fence".into());
            }
            println!(
                "recovery-fence: advanced-generation=pass late-output-rejected=pass parent-unchanged=pass"
            );
            Ok(())
        }
        "--cancel-recovery-fault-stdin" => cancel(client, read()?).await,
        "--capture-recovery-fault-cancelled-stdin" => capture(client, read()?, "cancelled").await,
        "--capture-recovery-fault-interrupted-stdin" => {
            capture(client, read()?, "interrupted").await
        }
        "--verify-recovery-fault-stdin" => {
            verify(client, &read()?).await?;
            println!(
                "recovery-fault: exact-replay=pass saved-work=pass parent-evidence=pass terminal-fence=pass attention=pass"
            );
            Ok(())
        }
        _ => Err("unknown recovery fault mode".into()),
    }
}
fn parent_request(start: &v1::StartTaskExecutionRequest) -> v1::GetTaskAttemptRequest {
    v1::GetTaskAttemptRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        task_id: start.task_id.clone(),
        attempt_id: start.attempt_id.clone(),
    }
}
fn child_request(e: &Evidence) -> Result<v1::GetTaskAttemptRequest> {
    let c = v1::ConfirmTaskRecoveryResponse::decode(e.confirmation.as_slice())?
        .confirmation
        .ok_or("confirmation missing")?;
    Ok(v1::GetTaskAttemptRequest {
        attempt_id: c.attempt_id,
        ..v1::GetTaskAttemptRequest::decode(e.parent_request.as_slice())?
    })
}
async fn feedback(
    client: &mut Client,
    selected: &v1::GetTaskAttemptRequest,
) -> Result<v1::ListTaskFeedbackResponse> {
    Ok(client
        .list_task_feedback(v1::ListTaskFeedbackRequest {
            workspace_id: selected.workspace_id.clone(),
            project_id: selected.project_id.clone(),
            conversation_id: selected.conversation_id.clone(),
            limit: 32,
            ..Default::default()
        })
        .await?
        .into_inner())
}
async fn confirm(client: &mut Client, seed: Seed) -> Result {
    let start = v1::StartTaskExecutionRequest::decode(seed.request.as_slice())?;
    let selected = parent_request(&start);
    let parent = client
        .get_task_attempt(selected.clone())
        .await?
        .into_inner();
    let item = feedback(client, &selected)
        .await?
        .feedback
        .into_iter()
        .find(|f| f.attempt_id == start.attempt_id && f.state == "open")
        .ok_or("open parent feedback missing")?;
    let record = client.record_task_recovery(v1::RecordTaskRecoveryRequest {
        workspace_id: start.workspace_id.clone(), project_id: start.project_id.clone(), conversation_id: start.conversation_id.clone(), task_id: start.task_id.clone(), attempt_id: start.attempt_id.clone(), idempotency_key: Uuid::now_v7().to_string(),
        content: Some(v1::record_task_recovery_request::Content::Proposal(v1::TaskRecoveryProposal { feedback_id: item.id, action: "continue".into(), explanation: "Continue accepted saved work under an explicit allowance; exercise interruption without a paid provider.".into(), additional_seconds: 300, additional_tokens: 2048, ..Default::default() })),
    }).await?.into_inner().record.ok_or("proposal missing")?;
    let request = v1::ConfirmTaskRecoveryRequest {
        workspace_id: start.workspace_id,
        project_id: start.project_id,
        conversation_id: start.conversation_id,
        task_id: start.task_id,
        proposal_id: record.id.clone(),
        proposal_digest: record.content_digest.clone(),
        idempotency_key: Uuid::now_v7().to_string(),
        acknowledge_provider_uncertainty: false,
    };
    let mut alias = request.clone();
    alias.idempotency_key = Uuid::now_v7().to_string();
    let mut second = client.clone();
    let (a, b) = tokio::join!(
        client.confirm_task_recovery(request.clone()),
        second.confirm_task_recovery(alias)
    );
    let receipt = a?.into_inner();
    if b?.into_inner() != receipt {
        return Err("confirmation race admitted different work".into());
    }
    let child = receipt.confirmation.as_ref().ok_or("child missing")?;
    let e = Evidence {
        parent_request: selected.encode_to_vec(),
        parent: parent.encode_to_vec(),
        proposal: record.encode_to_vec(),
        confirmation_request: request.encode_to_vec(),
        worker: format!("task-{}", child.attempt_id.replace('-', "")),
        confirmation: receipt.encode_to_vec(),
        cancellation_request: Vec::new(),
        cancellation: Vec::new(),
        terminal: Vec::new(),
        feedback: Vec::new(),
    };
    receipts(client, &e).await?;
    saved_work(client, &e).await?;
    emit(&e)
}
async fn receipts(client: &mut Client, e: &Evidence) -> Result {
    let request = v1::ConfirmTaskRecoveryRequest::decode(e.confirmation_request.as_slice())?;
    if client
        .confirm_task_recovery(request.clone())
        .await?
        .into_inner()
        .encode_to_vec()
        != e.confirmation
    {
        return Err("confirmation replay changed or admitted another attempt".into());
    }
    let mut alias = request;
    alias.idempotency_key = format!("replay-{}", alias.proposal_id);
    if client
        .confirm_task_recovery(alias.clone())
        .await?
        .into_inner()
        .encode_to_vec()
        != e.confirmation
    {
        return Err("alias replay changed recovery identity".into());
    }
    alias.proposal_digest = format!("sha256:{}", "0".repeat(64));
    if client.confirm_task_recovery(alias).await.is_ok() {
        return Err("changed proposal digest was accepted".into());
    }
    if !e.cancellation_request.is_empty()
        && client
            .cancel_task_execution(v1::CancelTaskExecutionRequest::decode(
                e.cancellation_request.as_slice(),
            )?)
            .await?
            .into_inner()
            .encode_to_vec()
            != e.cancellation
    {
        return Err("cancellation replay changed".into());
    }
    Ok(())
}
async fn saved_work(client: &mut Client, e: &Evidence) -> Result<v1::GetTaskAttemptResponse> {
    let old = v1::GetTaskAttemptResponse::decode(e.parent.as_slice())?
        .attempt
        .ok_or("parent missing")?;
    if client
        .get_task_attempt(v1::GetTaskAttemptRequest::decode(
            e.parent_request.as_slice(),
        )?)
        .await?
        .into_inner()
        .encode_to_vec()
        != e.parent
    {
        return Err("recovery changed accepted parent evidence".into());
    }
    let selected = child_request(e)?;
    let response = client
        .get_task_attempt(selected.clone())
        .await?
        .into_inner();
    let child = response
        .attempt
        .as_ref()
        .ok_or("continued attempt missing")?;
    if child.parent_attempt_id != old.attempt_id {
        return Err("continued attempt lost its parent link".into());
    }
    if child.saved_input_patch != old.patch
        || old.patch.is_empty()
        || child.source_commit != old.source_commit
        || child.runtime_image != old.runtime_image
        || child.configuration_revision_id != old.configuration_revision_id
        || child.max_seconds != 300
        || child.max_tokens != 2048
        || !child.patch.is_empty()
        || child.quality.is_some()
        || child.publication.is_some()
        || child.provider_state != "not_started"
    {
        return Err("continued input changed or unaccepted work became evidence".into());
    }
    let latest = client
        .get_task_execution(v1::GetTaskExecutionRequest {
            workspace_id: selected.workspace_id,
            project_id: selected.project_id,
            conversation_id: selected.conversation_id,
            task_id: selected.task_id,
        })
        .await?
        .into_inner()
        .preparation
        .ok_or("latest execution missing")?;
    if latest.attempt_id != child.attempt_id {
        return Err("recovery replay created a later attempt".into());
    }
    Ok(response)
}
async fn cancel(client: &mut Client, mut e: Evidence) -> Result {
    let s = child_request(&e)?;
    let request = v1::CancelTaskExecutionRequest {
        workspace_id: s.workspace_id,
        project_id: s.project_id,
        conversation_id: s.conversation_id,
        task_id: s.task_id,
        attempt_id: s.attempt_id,
        idempotency_key: Uuid::now_v7().to_string(),
    };
    e.cancellation = client
        .cancel_task_execution(request.clone())
        .await?
        .into_inner()
        .encode_to_vec();
    e.cancellation_request = request.encode_to_vec();
    receipts(client, &e).await?;
    saved_work(client, &e).await?;
    emit(&e)
}
async fn capture(client: &mut Client, mut e: Evidence, expected: &str) -> Result {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let response = saved_work(client, &e).await?;
        let state = &response.attempt.as_ref().ok_or("attempt missing")?.state;
        if state == expected {
            e.terminal = response.encode_to_vec();
            break;
        }
        if [
            "completed",
            "failed",
            "uncertain",
            "interrupted",
            "cancelled",
        ]
        .contains(&state.as_str())
            || tokio::time::Instant::now() > deadline
        {
            return Err(format!("expected recovery {expected}, observed {state}").into());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    e.feedback = feedback(client, &child_request(&e)?).await?.encode_to_vec();
    verify(client, &e).await?;
    emit(&e)
}
async fn verify(client: &mut Client, e: &Evidence) -> Result {
    receipts(client, e).await?;
    let response = saved_work(client, e).await?;
    if e.terminal.is_empty() || response.encode_to_vec() != e.terminal {
        return Err("terminal recovery evidence changed after interruption".into());
    }
    let selected = child_request(e)?;
    let page = feedback(client, &selected).await?;
    if page.encode_to_vec() != e.feedback {
        return Err("recovery attention changed after restart".into());
    }
    let confirmation = v1::ConfirmTaskRecoveryResponse::decode(e.confirmation.as_slice())?
        .confirmation
        .ok_or("confirmation missing")?;
    let parent = page
        .feedback
        .iter()
        .find(|f| f.attempt_id == confirmation.parent_attempt_id)
        .ok_or("parent notice missing")?;
    if parent.state != "resolved" || parent.recovery_attempt_id != selected.attempt_id {
        return Err("recovery lost the original attention decision".into());
    }
    let child = response.attempt.ok_or("child missing")?;
    let notices: Vec<_> = page
        .feedback
        .iter()
        .filter(|f| f.attempt_id == selected.attempt_id)
        .collect();
    if child.state == "interrupted" {
        if notices.len() != 1
            || notices[0].state != "open"
            || notices[0].safe_reason != "execution_interrupted"
        {
            return Err("interrupted recovery did not retain its own attention".into());
        }
    } else if child.state != "cancelled" || !notices.is_empty() {
        return Err("cancelled recovery produced unexpected attention".into());
    }
    let history = client
        .list_task_recovery(v1::ListTaskRecoveryRequest {
            workspace_id: selected.workspace_id,
            project_id: selected.project_id,
            conversation_id: selected.conversation_id,
            task_id: selected.task_id,
            attempt_id: confirmation.parent_attempt_id.clone(),
            limit: 32,
            ..Default::default()
        })
        .await?
        .into_inner();
    if history.records.len() != 1
        || history.records[0].encode_to_vec() != e.proposal
        || history.confirmations != vec![confirmation]
    {
        return Err("recovery proposal or once-only receipt changed".into());
    }
    Ok(())
}
