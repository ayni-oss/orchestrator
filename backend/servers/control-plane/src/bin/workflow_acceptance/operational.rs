//! Deployed capacity, isolated ownership and exact cancellation/restart evidence.
use super::{Client, Result, Scope, create_scope};
use ayni_orchestrator_api::v1::{self, project_service_client::ProjectServiceClient};
use prost::Message as _;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io::Read as _, time::Duration};
use tonic::transport::Channel;
use uuid::Uuid;

pub(super) async fn dispatch(
    mode: &str,
    client: &mut Client,
    projects: &mut ProjectServiceClient<Channel>,
) -> Result {
    match mode {
        "--prepare-operational-concurrency" => prepare(projects, client).await,
        "--capture-operational-concurrency-stdin" => capture(client).await,
        "--cancel-operational-concurrency-stdin" => cancel(client).await,
        "--verify-operational-concurrency-stdin" => verify(client).await,
        _ => Err("unknown operational acceptance mode".into()),
    }
}

const PACK: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../dev/fixtures/task-source/source.pack"
));
const META: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../dev/fixtures/task-source/source.json"
));
#[derive(Serialize, Deserialize)]
struct Evidence {
    scope: Scope,
    requests: Vec<Vec<u8>>,
    receipts: Vec<Vec<u8>>,
    blocked_dependency: Vec<u8>,
    #[serde(default)]
    active: Vec<Vec<u8>>,
    #[serde(default)]
    cancellation_requests: Vec<Vec<u8>>,
    #[serde(default)]
    cancellations: Vec<Vec<u8>>,
    #[serde(default)]
    terminal: Vec<Vec<u8>>,
}
fn key() -> String {
    Uuid::now_v7().to_string()
}
fn input() -> Result<Evidence> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("operational evidence exceeds bound".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn output(value: &Evidence) -> Result {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}
fn selected(scope: &Scope, request: &v1::StartTaskExecutionRequest) -> v1::GetTaskAttemptRequest {
    v1::GetTaskAttemptRequest {
        workspace_id: scope.workspace.clone(),
        project_id: scope.project.clone(),
        conversation_id: scope.conversation.clone(),
        task_id: request.task_id.clone(),
        attempt_id: request.attempt_id.clone(),
    }
}
fn inputs() -> Result<v1::TaskExecutionInputs> {
    let meta: serde_json::Value = serde_json::from_str(META)?;
    Ok(v1::TaskExecutionInputs {
        repository: "https://example.invalid/task-source.git".into(),
        source_ref: "refs/heads/main".into(),
        source_commit: meta["source_commit"]
            .as_str()
            .ok_or("source commit")?
            .into(),
        lock_digest: meta["lock_digest"].as_str().ok_or("lock digest")?.into(),
        runtime_image: format!("example.invalid/fixture@sha256:{}", "c".repeat(64)),
        max_seconds: 300,
        max_tokens: 10000,
        upstream_selections: vec![],
    })
}

pub async fn prepare(projects: &mut ProjectServiceClient<Channel>, client: &mut Client) -> Result {
    let scope = create_scope(projects, client).await?;
    let mut content = v1::PlanContent {
        title: "Concurrent isolated ownership".into(),
        objective: "Exercise four active slots with two queued tasks".into(),
        scope: "Explicit deterministic local acceptance fixture".into(),
        tasks: (0..7)
            .map(|n| v1::PlanTaskSpecification {
                key: format!("task{n}"),
                objective: "Task cancellation fixture".into(),
                scope: "One isolated checkout".into(),
                acceptance_criteria: vec!["Cancellation and receipt survive owner restart".into()],
                repository: "https://example.invalid/task-source.git".into(),
                source_ref: "refs/heads/main".into(),
                depends_on: if n == 6 {
                    vec!["task0".into(), "task1".into()]
                } else {
                    vec![]
                },
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let request = |content| v1::CreatePlanRequest {
        workspace_id: scope.workspace.clone(),
        project_id: scope.project.clone(),
        conversation_id: scope.conversation.clone(),
        idempotency_key: key(),
        content: Some(content),
        ..Default::default()
    };
    content.tasks[0].depends_on = vec!["task6".into()];
    if client.create_plan(request(content.clone())).await.is_ok() {
        return Err("cyclic dependency accepted".into());
    }
    content.tasks[0].depends_on.clear();
    let version = client
        .create_plan(request(content))
        .await?
        .into_inner()
        .version
        .ok_or("plan")?;
    let approved = client
        .approve_plan(v1::ApprovePlanRequest {
            workspace_id: scope.workspace.clone(),
            project_id: scope.project.clone(),
            conversation_id: scope.conversation.clone(),
            idempotency_key: key(),
            plan_id: version.plan_id,
            expected_version_id: version.id,
            content_digest: version.content_digest,
        })
        .await?
        .into_inner();
    let mut evidence = Evidence {
        scope: scope.clone(),
        requests: vec![],
        receipts: vec![],
        blocked_dependency: vec![],
        active: vec![],
        cancellation_requests: vec![],
        cancellations: vec![],
        terminal: vec![],
    };
    for (n, task) in approved.tasks.iter().enumerate() {
        let prepared = client
            .prepare_task_execution(v1::PrepareTaskExecutionRequest {
                workspace_id: scope.workspace.clone(),
                project_id: scope.project.clone(),
                conversation_id: scope.conversation.clone(),
                idempotency_key: key(),
                task_id: task.id.clone(),
                inputs: Some(inputs()?),
            })
            .await?
            .into_inner();
        let preparation = prepared.preparation.as_ref().ok_or("preparation")?;
        if n == 6 {
            if !preparation.attempt_id.is_empty() || preparation.blockers.len() != 2 {
                return Err("unselected dependencies became runnable".into());
            }
            evidence.blocked_dependency = prepared.encode_to_vec();
            continue;
        }
        if preparation.attempt_id.is_empty() || !preparation.blockers.is_empty() {
            return Err("independent task blocked".into());
        }
        let request = v1::StartTaskExecutionRequest {
            workspace_id: scope.workspace.clone(),
            project_id: scope.project.clone(),
            conversation_id: scope.conversation.clone(),
            task_id: task.id.clone(),
            attempt_id: preparation.attempt_id.clone(),
            idempotency_key: key(),
            source_pack: PACK.to_vec(),
        };
        let receipt = client
            .start_task_execution(request.clone())
            .await?
            .into_inner();
        evidence.requests.push(request.encode_to_vec());
        evidence.receipts.push(receipt.encode_to_vec());
    }
    output(&evidence)
}

pub async fn capture(client: &mut Client) -> Result {
    let mut evidence = input()?;
    if evidence.requests.len() != 6
        || evidence.receipts.len() != 6
        || evidence.blocked_dependency.is_empty()
    {
        return Err("incomplete capacity evidence".into());
    }
    for _ in 0..180 {
        let mut observations = Vec::new();
        for bytes in &evidence.requests {
            let request = v1::StartTaskExecutionRequest::decode(bytes.as_slice())?;
            observations.push(
                client
                    .get_task_attempt(selected(&evidence.scope, &request))
                    .await?
                    .into_inner(),
            );
        }
        let active = observations
            .iter()
            .filter_map(|response| response.attempt.as_ref())
            .filter(|a| a.state == "claimed" && !a.pod_uid.is_empty())
            .collect::<Vec<_>>();
        let queued = observations
            .iter()
            .filter_map(|response| response.attempt.as_ref())
            .filter(|a| a.state == "queued")
            .count();
        if active.len() == 4 && queued == 2 {
            let ids = active.iter().map(|a| &a.pod_uid).collect::<BTreeSet<_>>();
            let attempts = active
                .iter()
                .map(|a| &a.attempt_id)
                .collect::<BTreeSet<_>>();
            if ids.len() != 4 || attempts.len() != 4 || active.iter().any(|a| a.generation != 1) {
                return Err("concurrent ownership not isolated".into());
            }
            evidence.active = observations
                .iter()
                .map(|value| value.encode_to_vec())
                .collect();
            return output(&evidence);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("four isolated active tasks and two queued tasks were not observed".into())
}

pub async fn cancel(client: &mut Client) -> Result {
    let mut evidence = input()?;
    if evidence.active.len() != 6 {
        return Err("saturated capacity evidence required".into());
    }
    for bytes in &evidence.requests {
        let original = v1::StartTaskExecutionRequest::decode(bytes.as_slice())?;
        let request = v1::CancelTaskExecutionRequest {
            workspace_id: evidence.scope.workspace.clone(),
            project_id: evidence.scope.project.clone(),
            conversation_id: evidence.scope.conversation.clone(),
            task_id: original.task_id,
            attempt_id: original.attempt_id,
            idempotency_key: key(),
        };
        let receipt = client
            .cancel_task_execution(request.clone())
            .await?
            .into_inner();
        if client
            .cancel_task_execution(request.clone())
            .await?
            .into_inner()
            != receipt
        {
            return Err("capacity cancellation replay changed".into());
        }
        evidence.cancellation_requests.push(request.encode_to_vec());
        evidence.cancellations.push(receipt.encode_to_vec());
    }
    for _ in 0..180 {
        let mut terminal = Vec::new();
        for bytes in &evidence.requests {
            let request = v1::StartTaskExecutionRequest::decode(bytes.as_slice())?;
            terminal.push(
                client
                    .get_task_attempt(selected(&evidence.scope, &request))
                    .await?
                    .into_inner(),
            );
        }
        if terminal.iter().all(|response| {
            response
                .attempt
                .as_ref()
                .is_some_and(|a| a.state == "cancelled" && a.generation == 2 && a.patch.is_empty())
        }) {
            evidence.terminal = terminal.iter().map(|value| value.encode_to_vec()).collect();
            return output(&evidence);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("concurrent cancellation did not settle all six tasks".into())
}

pub async fn verify(client: &mut Client) -> Result {
    let evidence = input()?;
    if evidence.requests.len() != 6
        || evidence.terminal.len() != 6
        || evidence.cancellations.len() != 6
        || evidence.cancellation_requests.len() != 6
    {
        return Err("terminal capacity evidence incomplete".into());
    }
    for (index, bytes) in evidence.requests.iter().enumerate() {
        let request = v1::StartTaskExecutionRequest::decode(bytes.as_slice())?;
        if client
            .start_task_execution(request.clone())
            .await?
            .into_inner()
            .encode_to_vec()
            != evidence.receipts[index]
        {
            return Err("start receipt changed after restart".into());
        }
        if client
            .get_task_attempt(selected(&evidence.scope, &request))
            .await?
            .into_inner()
            .encode_to_vec()
            != evidence.terminal[index]
        {
            return Err("terminal authority changed after restart".into());
        }
        let cancel = v1::CancelTaskExecutionRequest::decode(
            evidence.cancellation_requests[index].as_slice(),
        )?;
        if client
            .cancel_task_execution(cancel)
            .await?
            .into_inner()
            .encode_to_vec()
            != evidence.cancellations[index]
        {
            return Err("cancel receipt changed after restart".into());
        }
    }
    println!(
        "operational-concurrency: four-active=pass two-queued=pass isolated-pod-identities=pass dependency-blockers=pass cyclic-rejection=pass cancellation=pass exact-replay=pass"
    );
    Ok(())
}
