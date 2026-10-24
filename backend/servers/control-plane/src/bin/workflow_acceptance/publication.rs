//! Real Git transport and explicit GitHub protocol fixture publication acceptance.
use super::{Client, Result};
use ayni_orchestrator_api::v1;
use ayni_orchestrator_events::task_publication::oid;
use prost::Message as _;
use serde::{Deserialize, Serialize};
use std::{io::Read as _, time::Duration};
use tonic::Code;

#[derive(Deserialize)]
struct CodingEvidence {
    request: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
struct Evidence {
    execution_request: Vec<u8>,
    publication_request: Vec<u8>,
    publication_receipt: Vec<u8>,
    publication_result: Vec<u8>,
}
fn input<T: serde::de::DeserializeOwned>() -> Result<T> {
    let mut bytes = Vec::new();
    std::io::stdin().take(262_145).read_to_end(&mut bytes)?;
    if bytes.len() > 262_144 {
        return Err("publication evidence limit".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn read_request(start: &v1::StartTaskExecutionRequest) -> v1::GetTaskAttemptRequest {
    v1::GetTaskAttemptRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        task_id: start.task_id.clone(),
        attempt_id: start.attempt_id.clone(),
    }
}
async fn attempt(
    client: &Client,
    start: &v1::StartTaskExecutionRequest,
) -> Result<v1::TaskAttempt> {
    let scoped_request = read_request(start);
    read_with_retry(
        || {
            let mut observer = client.clone();
            let request = scoped_request.clone();
            async move { observer.get_task_attempt(request).await }
        },
        Duration::from_secs(30),
    )
    .await?
    .into_inner()
    .attempt
    .ok_or_else(|| "publication attempt missing".into())
}
// This observer may outlive a local rollout. Retry only the exact read, never
// publication admission, permits, cancellation, or a changed scope.
async fn read_with_retry<T, F, Fut>(
    mut read: F,
    budget: Duration,
) -> std::result::Result<T, tonic::Status>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<T, tonic::Status>>,
{
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let result = tokio::time::timeout_at(deadline, read())
            .await
            .map_err(|_| {
                tonic::Status::unavailable("publication observer read deadline exceeded")
            })?;
        match result {
            Err(error) if error.code() == Code::Unavailable => {
                let retry_at = tokio::time::Instant::now() + Duration::from_millis(500);
                if retry_at >= deadline {
                    return Err(error);
                }
                tokio::time::sleep_until(retry_at).await;
            }
            result => return result,
        }
    }
}
async fn request(
    client: &Client,
    start: &v1::StartTaskExecutionRequest,
) -> Result<v1::StartTaskPublicationRequest> {
    let retained = attempt(client, start).await?;
    Ok(v1::StartTaskPublicationRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        task_id: start.task_id.clone(),
        attempt_id: start.attempt_id.clone(),
        idempotency_key: format!("publish-{}", start.attempt_id),
        expected_patch_digest: retained.patch_digest,
        expected_quality_digest: retained.quality_digest,
    })
}
pub async fn start_stdin(client: &mut Client, wait: bool) -> Result {
    let coding: CodingEvidence = input()?;
    let start = v1::StartTaskExecutionRequest::decode(coding.request.as_slice())?;
    let publication = request(client, &start).await?;
    let receipt = client
        .start_task_publication(publication.clone())
        .await?
        .into_inner();
    let mut evidence = Evidence {
        execution_request: coding.request,
        publication_request: publication.encode_to_vec(),
        publication_receipt: receipt.encode_to_vec(),
        publication_result: Vec::new(),
    };
    if wait {
        evidence.publication_result = terminal(client, &evidence, "draft").await?.encode_to_vec();
    }
    println!("{}", serde_json::to_string(&evidence)?);
    Ok(())
}
pub async fn reject_quality_stdin(client: &mut Client) -> Result {
    let coding: CodingEvidence = input()?;
    let start = v1::StartTaskExecutionRequest::decode(coding.request.as_slice())?;
    let publication = request(client, &start).await?;
    let status = client
        .start_task_publication(publication)
        .await
        .err()
        .ok_or("failed quality authorized publication")?;
    if status.code() != Code::FailedPrecondition
        || attempt(client, &start).await?.publication.is_some()
    {
        return Err("quality rejection created publication authority".into());
    }
    println!("publication-quality-gate: failed-quality-rejected=pass no-intent=pass");
    Ok(())
}
pub async fn verify_stdin(client: &mut Client, expected: &str) -> Result {
    let evidence: Evidence = input()?;
    let mut publication = terminal(client, &evidence, expected).await?;
    if !evidence.publication_result.is_empty() {
        let original = v1::TaskPublication::decode(evidence.publication_result.as_slice())?;
        if expected == "uncertain" {
            if publication.generation < original.generation {
                return Err("publication ownership generation moved backwards".into());
            }
            publication.generation = original.generation;
        }
        if publication != original {
            return Err("retained publication identity changed".into());
        }
    }
    println!(
        "publication: state={} retained-identity=pass exact-retry=pass real-git=true github-protocol-fixture=true live-github-auth=false",
        publication.state
    );
    Ok(())
}
pub async fn capture_stdin(client: &mut Client, expected: &str) -> Result {
    let mut evidence: Evidence = input()?;
    evidence.publication_result = terminal(client, &evidence, expected).await?.encode_to_vec();
    println!("{}", serde_json::to_string(&evidence)?);
    Ok(())
}
pub async fn reject_cancel_stdin(client: &mut Client) -> Result {
    let evidence: Evidence = input()?;
    let start = v1::StartTaskExecutionRequest::decode(evidence.execution_request.as_slice())?;
    let before = attempt(client, &start).await?;
    let status = client
        .cancel_task_publication(v1::CancelTaskPublicationRequest {
            workspace_id: start.workspace_id.clone(),
            project_id: start.project_id.clone(),
            conversation_id: start.conversation_id.clone(),
            task_id: start.task_id.clone(),
            attempt_id: start.attempt_id.clone(),
            idempotency_key: format!("reject-cancel-{}", start.attempt_id),
        })
        .await
        .err()
        .ok_or("consumed publication permit was cancellable")?;
    if status.code() != Code::FailedPrecondition || attempt(client, &start).await? != before {
        return Err("rejected cancellation changed retained publication".into());
    }
    println!("consumed-publication-cancel: rejected=pass retained-identity=pass");
    Ok(())
}
pub async fn inspect_stdin(client: &Client) -> Result {
    let evidence: Evidence = input()?;
    let start = v1::StartTaskExecutionRequest::decode(evidence.execution_request.as_slice())?;
    let publication = attempt(client, &start)
        .await?
        .publication
        .ok_or("publication missing")?;
    println!(
        "{}",
        serde_json::json!({
            "workspace_id":start.workspace_id,"generation":publication.generation,"publication_id":publication.publication_id,
            "state":publication.state,"branch":publication.branch,"base_branch":publication.base_branch,
            "commit_sha":publication.commit_sha,"tree_sha":publication.tree_sha,
            "pr_number":publication.pr_number,"pr_url":publication.pr_url,
            "safe_reason":publication.safe_reason,"pr_state":publication.pr_state,
        })
    );
    Ok(())
}
pub async fn cancel_stdin(client: &mut Client) -> Result {
    let evidence: Evidence = input()?;
    let start = v1::StartTaskExecutionRequest::decode(evidence.execution_request.as_slice())?;
    let cancel = v1::CancelTaskPublicationRequest {
        workspace_id: start.workspace_id.clone(),
        project_id: start.project_id.clone(),
        conversation_id: start.conversation_id.clone(),
        task_id: start.task_id.clone(),
        attempt_id: start.attempt_id.clone(),
        idempotency_key: format!("cancel-publication-{}", start.attempt_id),
    };
    let receipt = client
        .cancel_task_publication(cancel.clone())
        .await?
        .into_inner();
    if client.cancel_task_publication(cancel).await?.into_inner() != receipt {
        return Err("publication cancellation receipt changed".into());
    }
    terminal(client, &evidence, "cancelled").await?;
    println!("publication-cancellation: pre-create-fence=pass exact-retry=pass");
    Ok(())
}
async fn terminal(
    client: &mut Client,
    evidence: &Evidence,
    expected: &str,
) -> Result<v1::TaskPublication> {
    let start = v1::StartTaskExecutionRequest::decode(evidence.execution_request.as_slice())?;
    let publication =
        v1::StartTaskPublicationRequest::decode(evidence.publication_request.as_slice())?;
    if client
        .start_task_publication(publication.clone())
        .await?
        .into_inner()
        .encode_to_vec()
        != evidence.publication_receipt
    {
        return Err("publication start receipt changed".into());
    }
    for _ in 0..240 {
        let current = attempt(client, &start).await?;
        let result = current
            .publication
            .ok_or("publication projection missing")?;
        if result.state == expected {
            if result.patch_digest != publication.expected_patch_digest
                || result.quality_digest != publication.expected_quality_digest
                || current.execution_outcome != "completed"
                || current.quality.as_ref().is_none_or(|q| q.state != "passed")
            {
                return Err("publication changed frozen coding/quality identity".into());
            }
            if expected == "draft" {
                validate_draft(&result)?;
            }
            return Ok(result);
        }
        if matches!(
            result.state.as_str(),
            "blocked" | "cancelled" | "interrupted"
        ) {
            return Err(
                format!("publication ended {}: {}", result.state, result.safe_reason).into(),
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!("publication did not reach {expected}").into())
}
fn validate_draft(result: &v1::TaskPublication) -> Result {
    if !oid(&result.commit_sha)
        || !oid(&result.tree_sha)
        || result.pr_number == 0
        || result.branch.is_empty()
        || result.base_branch != "main"
        || result.repository != "https://github.com/fixture/repo.git"
        || result.pr_url != format!("https://github.com/fixture/repo/pull/{}", result.pr_number)
        || !matches!(result.pr_state.as_str(), "draft" | "closed" | "merged")
    {
        return Err("publication did not retain the exact fixture draft identity".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::read_with_retry;
    use std::{future::ready, time::Duration};
    use tonic::{Code, Status};

    #[tokio::test]
    async fn unavailable_read_recovers() -> std::result::Result<(), Status> {
        let mut calls = 0;
        let value = read_with_retry(
            || {
                calls += 1;
                ready(if calls == 1 {
                    Err(Status::unavailable("restart"))
                } else {
                    Ok(42)
                })
            },
            Duration::from_secs(2),
        )
        .await?;
        assert_eq!(value, 42);
        assert_eq!(calls, 2);
        Ok(())
    }

    #[tokio::test]
    async fn other_status_is_not_retried() {
        let mut calls = 0;
        let error = read_with_retry(
            || {
                calls += 1;
                ready(Err::<(), _>(Status::failed_precondition(
                    "authority changed",
                )))
            },
            Duration::from_secs(2),
        )
        .await;
        assert!(matches!(error, Err(status) if status.code() == Code::FailedPrecondition));
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn unavailable_does_not_exceed_budget() {
        let mut calls = 0;
        let error = read_with_retry(
            || {
                calls += 1;
                ready(Err::<(), _>(Status::unavailable("restart")))
            },
            Duration::from_millis(5),
        )
        .await;
        assert!(matches!(error, Err(status) if status.code() == Code::Unavailable));
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn stalled_read_is_bounded() {
        let error = read_with_retry(
            std::future::pending::<std::result::Result<(), Status>>,
            Duration::from_millis(5),
        )
        .await;
        assert!(matches!(error, Err(status) if status.code() == Code::Unavailable));
    }
}
