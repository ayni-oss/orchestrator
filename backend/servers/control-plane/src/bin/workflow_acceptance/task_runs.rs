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
    #[serde(default)]
    quality: bool,
    #[serde(default)]
    quality_failed: bool,
    #[serde(default)]
    quality_digest: String,
    #[serde(default)]
    feedback: Vec<u8>,
    #[serde(default)]
    recovery: Vec<u8>,
}
const PACK: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../dev/fixtures/task-source/source.pack"
));
const META: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../dev/fixtures/task-source/source.json"
));
pub async fn prepare_publication_quality(
    projects: &mut ProjectServiceClient<Channel>,
    client: &mut Client,
    failed: bool,
) -> Result {
    prepare_inner(
        projects,
        client,
        if failed {
            Profile::PublicationFailed
        } else {
            Profile::PublicationPassed
        },
    )
    .await
}
pub async fn prepare_quality(
    projects: &mut ProjectServiceClient<Channel>,
    client: &mut Client,
) -> Result {
    prepare_inner(projects, client, Profile::QualityPassed).await
}
pub async fn prepare_quality_failed(
    projects: &mut ProjectServiceClient<Channel>,
    client: &mut Client,
) -> Result {
    prepare_inner(projects, client, Profile::QualityFailed).await
}
pub async fn prepare(
    projects: &mut ProjectServiceClient<Channel>,
    client: &mut Client,
    pending: bool,
) -> Result {
    prepare_inner(
        projects,
        client,
        if pending {
            Profile::Pending
        } else {
            Profile::Coding
        },
    )
    .await
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum Profile {
    Coding,
    Pending,
    QualityPassed,
    QualityFailed,
    PublicationPassed,
    PublicationFailed,
}
async fn prepare_inner(
    projects: &mut ProjectServiceClient<Channel>,
    client: &mut Client,
    profile: Profile,
) -> Result {
    let pending = profile == Profile::Pending;
    let quality = !matches!(profile, Profile::Coding | Profile::Pending);
    let quality_failed = matches!(profile, Profile::QualityFailed | Profile::PublicationFailed);
    let scope = create_scope(projects, client).await?;
    let (meta, pack) = source_fixture(quality);
    let metadata: serde_json::Value = serde_json::from_str(meta)?;
    let created = client
        .create_plan(v1::CreatePlanRequest {
            workspace_id: scope.workspace.clone(),
            project_id: scope.project.clone(),
            conversation_id: scope.conversation.clone(),
            idempotency_key: Uuid::now_v7().to_string(),
            content: Some(plan_content(profile)),
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
                repository: fixture_repository(profile).into(),
                source_ref: "refs/heads/main".into(),
                source_commit: metadata["source_commit"]
                    .as_str()
                    .ok_or("commit missing")?
                    .into(),
                lock_digest: metadata["lock_digest"]
                    .as_str()
                    .ok_or("lock missing")?
                    .into(),
                runtime_image: if quality {
                    std::env::var("ORCHESTRATOR_TASK_QUALITY_RUNTIME_IMAGE")?
                } else {
                    format!("example.invalid/fixture@sha256:{}", "c".repeat(64))
                },
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
        source_pack: pack.to_vec(),
    };
    let receipt = client
        .start_task_execution(request.clone())
        .await?
        .into_inner();
    let mut evidence = Evidence {
        scope,
        request: request.encode_to_vec(),
        receipt: receipt.encode_to_vec(),
        quality,
        quality_failed,
        quality_digest: String::new(),
        feedback: Vec::new(),
        recovery: Vec::new(),
    };
    if !pending {
        let attempt = verify(client, &evidence).await?;
        evidence.feedback = feedback(client, &request, expected_feedback(&evidence)).await?;
        evidence.recovery = super::recovery::record(client, &request).await?;
        if let Some(quality) = attempt.quality {
            evidence.quality_digest =
                ayni_orchestrator_events::task_execution::digest(&quality.encode_to_vec());
        }
    }
    println!("{}", serde_json::to_string(&evidence)?);
    Ok(())
}
pub async fn verify_quality_stdin(client: &mut Client) -> Result {
    let evidence = stdin_evidence()?;
    if !evidence.quality {
        return Err("quality acceptance evidence required".into());
    }
    verify(client, &evidence).await?;
    println!(
        "task-quality: actual-worker-runtime=pass managed-evidence=pass frozen-inputs=pass complete-accounting=pass retained-after-restart=pass coding-fixture=true"
    );
    Ok(())
}
pub async fn verify_quality_failed_stdin(client: &mut Client) -> Result {
    let evidence = stdin_evidence()?;
    if !evidence.quality || !evidence.quality_failed {
        return Err("failed quality acceptance evidence required".into());
    }
    verify(client, &evidence).await?;
    println!(
        "task-quality-failure: coding-completed=pass quality-failed=pass complete-accounting=pass retained-after-restart=pass coding-fixture=true"
    );
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
async fn verify(client: &mut Client, evidence: &Evidence) -> Result<v1::TaskAttempt> {
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
    let expected_patch = if evidence.quality_failed {
        "src/lib.rs"
    } else {
        "task-result.txt"
    };
    if !attempt.patch.contains(expected_patch)
        || (!evidence.quality_failed
            && !attempt.patch.contains("+Task execution fixture completed."))
        || attempt.pod_uid.is_empty()
    {
        return Err("task did not retain isolated changes and workload identity".into());
    }
    let meta = if evidence.quality {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../dev/fixtures/task-quality/source.json"
        ))
    } else {
        META
    };
    let metadata: serde_json::Value = serde_json::from_str(meta)?;
    verify_quality_outcome(evidence, &attempt, &metadata)?;
    if Some(attempt.source_commit.as_str()) != metadata["source_commit"].as_str() {
        return Err("source commit changed".into());
    }
    verify_retained_quality(evidence, &attempt)?;
    let retained = feedback(
        client,
        &v1::StartTaskExecutionRequest::decode(evidence.request.as_slice())?,
        expected_feedback(evidence),
    )
    .await?;
    if !evidence.feedback.is_empty() && retained != evidence.feedback {
        return Err("retained task feedback changed after restart".into());
    }
    if !evidence.recovery.is_empty() {
        let start = v1::StartTaskExecutionRequest::decode(evidence.request.as_slice())?;
        if super::recovery::read(client, &start).await? != evidence.recovery {
            return Err("recovery records changed after restart".into());
        }
    }
    Ok(attempt)
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
pub async fn cancel_stdin(client: &mut Client, wait: bool) -> Result {
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
    if wait {
        terminal(client, &start, "cancelled").await?;
        println!(
            "task-cancellation: real-worker-stop=pass exact-retry=pass fence=pass retained-storage=pass"
        );
    } else {
        println!("task-cancellation: fence-recorded=pass exact-retry=pass");
    }
    Ok(())
}
pub async fn fenced_stdin(client: &mut Client) -> Result {
    let evidence = stdin_evidence()?;
    let start = v1::StartTaskExecutionRequest::decode(evidence.request.as_slice())?;
    let attempt = client
        .get_task_attempt(v1::GetTaskAttemptRequest {
            workspace_id: start.workspace_id.clone(),
            project_id: start.project_id.clone(),
            conversation_id: start.conversation_id.clone(),
            task_id: start.task_id.clone(),
            attempt_id: start.attempt_id.clone(),
        })
        .await?
        .into_inner()
        .attempt
        .ok_or("fenced attempt missing")?;
    if attempt.state != "cancelling"
        || attempt.generation <= 1
        || !attempt.patch.is_empty()
        || attempt.quality.is_some()
    {
        return Err("late worker changed cancelled execution evidence".into());
    }
    if client
        .start_task_execution(start)
        .await?
        .into_inner()
        .encode_to_vec()
        != evidence.receipt
    {
        return Err("fenced start replay changed".into());
    }
    println!(
        "task-fence: advanced-generation=pass late-report-not-accepted=pass original-start-receipt=pass"
    );
    Ok(())
}
pub async fn interrupted_stdin(client: &mut Client) -> Result {
    let evidence = stdin_evidence()?;
    let start = v1::StartTaskExecutionRequest::decode(evidence.request.as_slice())?;
    terminal(client, &start, "interrupted").await?;
    let retained_feedback = feedback(client, &start, Some("execution_interrupted")).await?;
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
    println!(
        "task-feedback-sha256: {}",
        ayni_orchestrator_events::task_execution::digest(&retained_feedback)
    );
    Ok(())
}

const fn expected_feedback(evidence: &Evidence) -> Option<&'static str> {
    if evidence.quality_failed {
        Some("quality_failed")
    } else if evidence.quality {
        None
    } else {
        // The coding-only fixture intentionally cannot perform managed quality.
        Some("quality_blocked")
    }
}

async fn feedback(
    client: &mut Client,
    start: &v1::StartTaskExecutionRequest,
    expected: Option<&str>,
) -> Result<Vec<u8>> {
    let request = v1::ListTaskFeedbackRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        limit: 32,
        ..Default::default()
    };
    let page = client
        .list_task_feedback(request.clone())
        .await?
        .into_inner();
    if let Some(reason) = expected {
        if page.feedback.len() != 1
            || page.feedback[0].safe_reason != reason
            || page.feedback[0].task_id != start.task_id
            || page.feedback[0].attempt_id != start.attempt_id
            || page.feedback[0].conversation_id != start.conversation_id
            || page.feedback[0].state != "open"
        {
            return Err("actionable feedback missing or duplicated at immutable origin".into());
        }
    } else if !page.feedback.is_empty() {
        return Err("successful task created actionable feedback".into());
    }
    let project = client
        .list_task_feedback(v1::ListTaskFeedbackRequest {
            conversation_id: String::new(),
            ..request.clone()
        })
        .await?
        .into_inner();
    if project != page {
        return Err("project attention differs from originating conversation".into());
    }
    if client.list_task_feedback(request).await?.into_inner() != page {
        return Err("repeated feedback read changed".into());
    }
    Ok(page.encode_to_vec())
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

const fn source_fixture(quality: bool) -> (&'static str, &'static [u8]) {
    let meta = if quality {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../dev/fixtures/task-quality/source.json"
        ))
    } else {
        META
    };
    let pack: &[u8] = if quality {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../dev/fixtures/task-quality/source.pack"
        ))
    } else {
        PACK
    };
    (meta, pack)
}

const fn fixture_repository(profile: Profile) -> &'static str {
    if matches!(
        profile,
        Profile::PublicationPassed | Profile::PublicationFailed
    ) {
        "https://github.com/fixture/repo.git"
    } else {
        "https://example.invalid/task-source.git"
    }
}

fn plan_content(profile: Profile) -> v1::PlanContent {
    v1::PlanContent {
        title: "Isolated coding fixture".into(),
        objective: "Retain isolated task changes".into(),
        scope: "Task execution acceptance".into(),
        tasks: vec![v1::PlanTaskSpecification {
            key: "coding".into(),
            objective: if matches!(profile, Profile::QualityFailed | Profile::PublicationFailed) {
                "Task quality failure fixture"
            } else if profile == Profile::Pending {
                "Task cancellation fixture"
            } else {
                "Produce the deterministic fixture file"
            }
            .into(),
            scope: "Only this checkout".into(),
            acceptance_criteria: vec!["Retained patch survives restart".into()],
            repository: fixture_repository(profile).into(),
            source_ref: "refs/heads/main".into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn verify_retained_quality(evidence: &Evidence, attempt: &v1::TaskAttempt) -> Result {
    if !evidence.quality_digest.is_empty() {
        let quality = attempt.quality.as_ref().ok_or("retained quality missing")?;
        if ayni_orchestrator_events::task_execution::digest(&quality.encode_to_vec())
            != evidence.quality_digest
        {
            return Err("retained quality evidence changed".into());
        }
    }
    Ok(())
}

fn verify_quality_outcome(
    evidence: &Evidence,
    attempt: &v1::TaskAttempt,
    metadata: &serde_json::Value,
) -> Result {
    if evidence.quality {
        let quality = attempt.quality.as_ref().ok_or("quality outcome missing")?;
        let expected_state = if evidence.quality_failed {
            "failed"
        } else {
            "passed"
        };
        if attempt.execution_outcome != "completed"
            || quality.state != expected_state
            || (evidence.quality_failed && quality.safe_reason != "quality_failed")
            || (evidence.quality_failed && quality.exit_code == Some(0))
            || (!evidence.quality_failed && quality.exit_code != Some(0))
            || quality.expected_targets == 0
            || quality.expected_targets != quality.completed_targets
            || quality.artifact_digest
                != ayni_orchestrator_events::task_execution::digest(quality.artifact.as_bytes())
            || Some(quality.lock_digest.as_str()) != metadata["lock_digest"].as_str()
            || quality.runtime_image != attempt.runtime_image
        {
            return Err(format!(
                "quality fixture did not pass managed validation: {} {}",
                quality.state, quality.safe_reason
            )
            .into());
        }
    }
    Ok(())
}
