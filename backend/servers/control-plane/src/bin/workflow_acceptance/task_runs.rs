//! Real Kubernetes coding with an explicitly enabled deterministic worker fixture.
use super::{Client, Result, Scope, create_scope};
use ayni_orchestrator_api::v1::{self, project_service_client::ProjectServiceClient};
use prost::Message as _;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tonic::transport::Channel;
use uuid::Uuid;
#[derive(Serialize, Deserialize)]
struct Evidence {
    scope: Scope,
    request: Vec<u8>,
    receipt: Vec<u8>,
}
const PACK: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../dev/fixtures/task-source/source.pack"
));
const META: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../dev/fixtures/task-source/source.json"
));
pub async fn prepare(
    projects: &mut ProjectServiceClient<Channel>,
    client: &mut Client,
    pending: bool,
) -> Result {
    let scope = create_scope(projects, client).await?;
    let metadata: serde_json::Value = serde_json::from_str(META)?;
    let created = client
        .create_plan(v1::CreatePlanRequest {
            workspace_id: scope.workspace.clone(),
            project_id: scope.project.clone(),
            conversation_id: scope.conversation.clone(),
            idempotency_key: Uuid::now_v7().to_string(),
            content: Some(v1::PlanContent {
                title: "Isolated coding fixture".into(),
                objective: "Retain isolated task changes".into(),
                scope: "Task execution acceptance".into(),
                tasks: vec![v1::PlanTaskSpecification {
                    key: "coding".into(),
                    objective: if pending {
                        "Task cancellation fixture"
                    } else {
                        "Produce the deterministic fixture file"
                    }
                    .into(),
                    scope: "Only this checkout".into(),
                    acceptance_criteria: vec!["Retained patch survives restart".into()],
                    repository: "https://example.invalid/task-source.git".into(),
                    source_ref: "refs/heads/main".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        })
        .await?
        .into_inner()
        .version
        .ok_or("version missing")?;
    let approved = client
        .approve_plan(v1::ApprovePlanRequest {
            workspace_id: scope.workspace.clone(),
            project_id: scope.project.clone(),
            conversation_id: scope.conversation.clone(),
            idempotency_key: Uuid::now_v7().to_string(),
            plan_id: created.plan_id,
            expected_version_id: created.id,
            content_digest: created.content_digest,
        })
        .await?
        .into_inner();
    let task = approved.tasks.first().ok_or("task missing")?;
    let prepared = client
        .prepare_task_execution(v1::PrepareTaskExecutionRequest {
            workspace_id: scope.workspace.clone(),
            project_id: scope.project.clone(),
            conversation_id: scope.conversation.clone(),
            task_id: task.id.clone(),
            idempotency_key: Uuid::now_v7().to_string(),
            inputs: Some(v1::TaskExecutionInputs {
                repository: "https://example.invalid/task-source.git".into(),
                source_ref: "refs/heads/main".into(),
                source_commit: metadata["source_commit"]
                    .as_str()
                    .ok_or("commit missing")?
                    .into(),
                lock_digest: metadata["lock_digest"]
                    .as_str()
                    .ok_or("lock missing")?
                    .into(),
                runtime_image: format!("example.invalid/fixture@sha256:{}", "c".repeat(64)),
                max_seconds: 300,
                max_tokens: 10000,
            }),
        })
        .await?
        .into_inner()
        .preparation
        .ok_or("preparation missing")?;
    let request = v1::StartTaskExecutionRequest {
        workspace_id: scope.workspace.clone(),
        project_id: scope.project.clone(),
        conversation_id: scope.conversation.clone(),
        task_id: task.id.clone(),
        attempt_id: prepared.attempt_id,
        idempotency_key: Uuid::now_v7().to_string(),
        source_pack: PACK.to_vec(),
    };
    let receipt = client
        .start_task_execution(request.clone())
        .await?
        .into_inner();
    let evidence = Evidence {
        scope,
        request: request.encode_to_vec(),
        receipt: receipt.encode_to_vec(),
    };
    if !pending {
        verify(client, &evidence).await?;
    }
    println!("{}", serde_json::to_string(&evidence)?);
    Ok(())
}
pub async fn verify_stdin(client: &mut Client) -> Result {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::io::stdin().take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err("task evidence bound exceeded".into());
    }
    verify(client, &serde_json::from_slice(&bytes)?).await?;
    println!(
        "task-execution: isolated-checkout=pass retained-patch=pass exact-start-retry=pass restart=pass fixture-only=true"
    );
    Ok(())
}
async fn verify(client: &mut Client, evidence: &Evidence) -> Result {
    let start = v1::StartTaskExecutionRequest::decode(evidence.request.as_slice())?;
    let receipt = client
        .start_task_execution(start.clone())
        .await?
        .into_inner();
    if receipt.encode_to_vec() != evidence.receipt {
        return Err("start receipt changed".into());
    }
    let read = v1::GetTaskAttemptRequest {
        workspace_id: start.workspace_id,
        project_id: start.project_id,
        conversation_id: start.conversation_id,
        task_id: start.task_id,
        attempt_id: start.attempt_id,
    };
    let mut completed = None;
    for _ in 0..240 {
        let result = client
            .get_task_attempt(read.clone())
            .await?
            .into_inner()
            .attempt
            .ok_or("attempt missing")?;
        match result.state.as_str() {
            "completed" => {
                completed = Some(result);
                break;
            }
            "failed" | "uncertain" | "interrupted" | "cancelled" => {
                return Err(format!(
                    "task fixture ended {}: {}",
                    result.state, result.safe_reason
                )
                .into());
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let attempt = completed.ok_or("task completion deadline")?;
    if !attempt.patch.contains("task-result.txt")
        || !attempt.patch.contains("+Task execution fixture completed.")
        || attempt.pod_uid.is_empty()
    {
        return Err("task did not retain isolated changes and workload identity".into());
    }
    let metadata: serde_json::Value = serde_json::from_str(META)?;
    if Some(attempt.source_commit.as_str()) != metadata["source_commit"].as_str() {
        return Err("source commit changed".into());
    }
    Ok(())
}

fn stdin_evidence() -> Result<Evidence> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::io::stdin().take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err("task evidence bound exceeded".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
pub async fn cancel_stdin(client: &mut Client) -> Result {
    let evidence = stdin_evidence()?;
    let start = v1::StartTaskExecutionRequest::decode(evidence.request.as_slice())?;
    let request = v1::CancelTaskExecutionRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        task_id: start.task_id.clone(),
        attempt_id: start.attempt_id.clone(),
        idempotency_key: format!("cancel-{}", start.attempt_id),
    };
    let receipt = client
        .cancel_task_execution(request.clone())
        .await?
        .into_inner();
    if client.cancel_task_execution(request).await?.into_inner() != receipt {
        return Err("cancel receipt changed".into());
    }
    terminal(client, &start, "cancelled").await?;
    println!(
        "task-cancellation: real-worker-stop=pass exact-retry=pass fence=pass retained-storage=pass"
    );
    Ok(())
}
pub async fn interrupted_stdin(client: &mut Client) -> Result {
    let evidence = stdin_evidence()?;
    let start = v1::StartTaskExecutionRequest::decode(evidence.request.as_slice())?;
    terminal(client, &start, "interrupted").await?;
    if client
        .start_task_execution(start)
        .await?
        .into_inner()
        .encode_to_vec()
        != evidence.receipt
    {
        return Err("interrupted start replay changed".into());
    }
    println!("task-pod-loss: interrupted=pass no-automatic-retry=pass retained-storage=pass");
    Ok(())
}
async fn terminal(
    client: &mut Client,
    start: &v1::StartTaskExecutionRequest,
    expected: &str,
) -> Result {
    let read = v1::GetTaskAttemptRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        task_id: start.task_id.clone(),
        attempt_id: start.attempt_id.clone(),
    };
    for _ in 0..120 {
        let attempt = client
            .get_task_attempt(read.clone())
            .await?
            .into_inner()
            .attempt
            .ok_or("attempt missing")?;
        if attempt.state == expected && !attempt.pod_uid.is_empty() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!("task did not reach {expected}").into())
}

pub fn name_stdin() -> Result {
    let evidence = stdin_evidence()?;
    let start = v1::StartTaskExecutionRequest::decode(evidence.request.as_slice())?;
    println!("task-{}", Uuid::parse_str(&start.attempt_id)?.simple());
    Ok(())
}
