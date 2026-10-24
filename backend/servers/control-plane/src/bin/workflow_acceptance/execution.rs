//! Fixture-only execution preparation; never claims image or worker compatibility.
use super::{Client, Result, Scope, create_scope, snapshot};
use ayni_orchestrator_api::v1::{self, project_service_client::ProjectServiceClient};
use prost::Message as _;
use serde::{Deserialize, Serialize};
use tonic::{Code, transport::Channel};
use uuid::Uuid;
#[derive(Serialize, Deserialize)]
struct Evidence {
    scope: Scope,
    request: Vec<u8>,
    prepared: Vec<u8>,
}
pub async fn prepare(projects: &mut ProjectServiceClient<Channel>, client: &mut Client) -> Result {
    let scope = create_scope(projects, client).await?;
    let plan = client
        .create_plan(v1::CreatePlanRequest {
            workspace_id: scope.workspace.clone(),
            project_id: scope.project.clone(),
            conversation_id: scope.conversation.clone(),
            idempotency_key: Uuid::now_v7().to_string(),
            content: Some(v1::PlanContent {
                title: "Execution preparation fixture".into(),
                objective: "Freeze inputs without dispatch".into(),
                scope: "Preparation only".into(),
                tasks: vec![v1::PlanTaskSpecification {
                    key: "first".into(),
                    objective: "Prepare one task".into(),
                    scope: "Fixture only".into(),
                    acceptance_criteria: vec!["Original preparation survives restart".into()],
                    repository: "https://example.invalid/task-fixture.git".into(),
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
            plan_id: plan.plan_id,
            expected_version_id: plan.id,
            content_digest: plan.content_digest,
        })
        .await?
        .into_inner();
    let task = approved.tasks.first().ok_or("task missing")?;
    let mut request = v1::PrepareTaskExecutionRequest {
        workspace_id: scope.workspace.clone(),
        project_id: scope.project.clone(),
        conversation_id: scope.conversation.clone(),
        task_id: task.id.clone(),
        idempotency_key: Uuid::now_v7().to_string(),
        inputs: Some(v1::TaskExecutionInputs::default()),
    };
    let blocked = client
        .prepare_task_execution(request.clone())
        .await?
        .into_inner();
    let preparation = blocked
        .preparation
        .as_ref()
        .ok_or("blocked result missing")?;
    if !preparation.attempt_id.is_empty() || preparation.blockers.is_empty() {
        return Err("missing inputs did not retain blockers".into());
    }
    if client
        .prepare_task_execution(request.clone())
        .await?
        .into_inner()
        != blocked
    {
        return Err("blocked receipt replay changed".into());
    }
    request.idempotency_key = Uuid::now_v7().to_string();
    request.inputs = Some(v1::TaskExecutionInputs {
        repository: task
            .specification
            .as_ref()
            .ok_or("specification missing")?
            .repository
            .clone(),
        source_ref: "refs/heads/main".into(),
        source_commit: "a".repeat(40),
        lock_digest: format!("sha256:{}", "b".repeat(64)),
        runtime_image: format!("example.invalid/fixture@sha256:{}", "c".repeat(64)),
        max_seconds: 600,
        max_tokens: 10000,
        upstream_selections: vec![],
    });
    let prepared = client
        .prepare_task_execution(request.clone())
        .await?
        .into_inner();
    let evidence = Evidence {
        scope,
        request: request.encode_to_vec(),
        prepared: prepared.encode_to_vec(),
    };
    verify(client, &evidence).await?;
    println!("{}", serde_json::to_string(&evidence)?);
    Ok(())
}
pub async fn verify_stdin(client: &mut Client) -> Result {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::io::stdin().take(32769).read_to_end(&mut bytes)?;
    if bytes.len() > 32768 {
        return Err("execution evidence exceeds bound".into());
    }
    verify(client, &serde_json::from_slice(&bytes)?).await?;
    println!(
        "execution-preparation: exact-retry=pass scoped-read=pass conflicting-key=pass retained-restart=pass no-dispatch=pass fixture-only=true"
    );
    Ok(())
}
async fn verify(client: &mut Client, evidence: &Evidence) -> Result {
    let request = v1::PrepareTaskExecutionRequest::decode(evidence.request.as_slice())?;
    let original = v1::PrepareTaskExecutionResponse::decode(evidence.prepared.as_slice())?;
    let prepared = original.preparation.as_ref().ok_or("preparation missing")?;
    if prepared.attempt_id.is_empty() || !prepared.blockers.is_empty() {
        return Err("attempt not prepared".into());
    }
    if client
        .prepare_task_execution(request.clone())
        .await?
        .into_inner()
        != original
    {
        return Err("exact receipt changed".into());
    }
    let read = client
        .get_task_execution(v1::GetTaskExecutionRequest {
            workspace_id: request.workspace_id.clone(),
            project_id: request.project_id.clone(),
            conversation_id: request.conversation_id.clone(),
            task_id: request.task_id.clone(),
        })
        .await?
        .into_inner();
    if read.preparation != original.preparation {
        return Err("retained preparation changed".into());
    }
    let mut competing = request.clone();
    competing.idempotency_key = Uuid::now_v7().to_string();
    if client
        .prepare_task_execution(competing)
        .await
        .err()
        .is_none_or(|s| s.code() != Code::AlreadyExists)
    {
        return Err("second attempt admitted".into());
    }
    let state = snapshot(client, &evidence.scope).await?;
    if state.conversation_revision != prepared.conversation_revision
        || state.resume_cursor != prepared.resume_cursor
        || !state.nonterminal_turns.is_empty()
        || !state.recent_terminal_turns.is_empty()
        || state
            .history_counts
            .as_ref()
            .is_none_or(|h| h.messages != 0 || h.turns != 0)
    {
        return Err("preparation dispatched work or changed observation".into());
    }
    Ok(())
}
