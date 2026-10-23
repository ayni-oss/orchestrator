//! Trusted local publication review; frozen JSON is replayed exactly on submit.
use ayni_orchestrator_api::v1::{
    self, project_conversation_service_client::ProjectConversationServiceClient,
};
use ayni_orchestrator_events::{task_execution::TaskCommand, task_publication};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs::{File, OpenOptions},
    io::{Read as _, Write as _},
    net::SocketAddr,
    process::ExitCode,
    time::Duration,
};
use tonic::transport::Endpoint;
type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frozen {
    request: v1::StartTaskPublicationRequest,
    title: String,
    body: String,
    repository: String,
    source_ref: String,
    source_commit: String,
    branch: String,
    base_branch: String,
    author_name: String,
    author_email: String,
    commit_message: String,
}
#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("task publication failed: {error}");
            ExitCode::FAILURE
        }
    }
}
async fn run() -> Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        return Err("usage: workflow-publish-task prepare LOOPBACK_ADDR SELECTION_JSON FROZEN_JSON | submit LOOPBACK_ADDR FROZEN_JSON | cancel LOOPBACK_ADDR CANCEL_JSON".into());
    }
    let address: SocketAddr = args[1].parse()?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("trusted loopback required".into());
    }
    let channel = Endpoint::from_shared(format!("http://{address}"))?
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .connect()
        .await?;
    let mut client = ProjectConversationServiceClient::new(channel);
    match args[0].as_str() {
        "prepare" if args.len() == 4 => prepare(&mut client, &args[2], &args[3]).await,
        "submit" if args.len() == 3 => submit(&mut client, &args[2]).await,
        "cancel" if args.len() == 3 => {
            let request: v1::CancelTaskPublicationRequest = read(&args[2])?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &client.cancel_task_publication(request).await?.into_inner()
                )?
            );
            Ok(())
        }
        _ => Err("invalid publication command".into()),
    }
}
fn read<T: serde::de::DeserializeOwned>(path: &str) -> Result<T> {
    let mut bytes = Vec::new();
    File::open(path)?.take(131_073).read_to_end(&mut bytes)?;
    if bytes.len() > 131_072 {
        return Err("publication file exceeds128KiB".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
async fn prepare(
    client: &mut ProjectConversationServiceClient<tonic::transport::Channel>,
    selection: &str,
    output: &str,
) -> Result {
    let selected = read(selection)?;
    let key = uuid::Uuid::now_v7().to_string();
    let frozen = review(client, selected, &key).await?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&serde_json::to_vec_pretty(&frozen)?)?;
    file.sync_all()?;
    println!(
        "Frozen publication request written. Review title, body, repository, source and evidence, then submit this exact file; preserve it for retries."
    );
    Ok(())
}

async fn review(
    client: &mut ProjectConversationServiceClient<tonic::transport::Channel>,
    selected: v1::GetTaskAttemptRequest,
    key: &str,
) -> Result<Frozen> {
    let attempt = client
        .get_task_attempt(selected.clone())
        .await?
        .into_inner()
        .attempt
        .ok_or("attempt missing")?;
    if attempt.state != "completed"
        || attempt.execution_outcome != "completed"
        || attempt.quality.as_ref().is_none_or(|q| q.state != "passed")
        || attempt.patch.is_empty()
    {
        return Err("completed changes and passing managed quality required".into());
    }
    let task = client
        .get_task(v1::GetTaskRequest {
            workspace_id: selected.workspace_id.clone(),
            project_id: selected.project_id.clone(),
            conversation_id: selected.conversation_id.clone(),
            task_id: selected.task_id.clone(),
        })
        .await?
        .into_inner()
        .task
        .ok_or("task missing")?
        .specification
        .ok_or("task specification missing")?;
    let command = TaskCommand {
        task_id: selected.task_id.clone(),
        attempt_id: selected.attempt_id.clone(),
        source_commit: attempt.source_commit.clone(),
        objective: task.objective,
        scope: task.scope,
        acceptance_criteria: task.acceptance_criteria,
        ..Default::default()
    };
    let frozen = Frozen {
        title: task_publication::title(&command.objective),
        body: task_publication::body(
            &command,
            &attempt.quality_digest,
            &attempt.patch_digest,
            &selected.attempt_id,
        ),
        repository: task_publication::github_repository(&task.repository)
            .ok_or("unsupported publication repository")?,
        base_branch: task_publication::base_branch(&task.source_ref)
            .ok_or("unsupported publication base")?,
        author_name: "Ayni Orchestrator".into(),
        author_email: "orchestrator@ayni.build".into(),
        commit_message: format!(
            "{}\n\nSigned-off-by: Ayni Orchestrator <orchestrator@ayni.build>\nai-generated: true\n",
            task_publication::title(&command.objective)
        ),
        source_ref: task.source_ref,
        source_commit: attempt.source_commit,
        branch: format!("task/{}", selected.attempt_id),
        request: v1::StartTaskPublicationRequest {
            workspace_id: selected.workspace_id,
            project_id: selected.project_id,
            conversation_id: selected.conversation_id,
            task_id: selected.task_id,
            attempt_id: selected.attempt_id,
            idempotency_key: key.into(),
            expected_patch_digest: attempt.patch_digest,
            expected_quality_digest: attempt.quality_digest,
        },
    };
    if frozen.body.len() > 24 * 1024 {
        return Err("approved publication metadata exceeds24KiB profile".into());
    }
    Ok(frozen)
}
async fn submit(
    client: &mut ProjectConversationServiceClient<tonic::transport::Channel>,
    path: &str,
) -> Result {
    let frozen: Frozen = read(path)?;
    let request = &frozen.request;
    let selected = v1::GetTaskAttemptRequest {
        workspace_id: request.workspace_id.clone(),
        project_id: request.project_id.clone(),
        conversation_id: request.conversation_id.clone(),
        task_id: request.task_id.clone(),
        attempt_id: request.attempt_id.clone(),
    };
    let canonical = review(client, selected, &request.idempotency_key).await?;
    compare_preview(&frozen, &canonical)?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &client
                .start_task_publication(frozen.request)
                .await?
                .into_inner()
        )?
    );
    Ok(())
}
fn compare_preview(frozen: &Frozen, canonical: &Frozen) -> Result {
    if frozen != canonical {
        return Err(
            "frozen publication preview differs from immutable reviewed task evidence".into(),
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edited_preview_or_reviewed_digests_cannot_authorize_different_publication() {
        let original = Frozen {
            request: v1::StartTaskPublicationRequest::default(),
            title: "feat(tasks): reviewed task".into(),
            body: "approved criteria".into(),
            repository: "https://github.com/example/repo.git".into(),
            source_ref: "refs/heads/alpha".into(),
            source_commit: "a".repeat(40),
            branch: "task/attempt".into(),
            base_branch: "alpha".into(),
            author_name: "Ayni Orchestrator".into(),
            author_email: "orchestrator@ayni.build".into(),
            commit_message: "reviewed commit".into(),
        };
        assert!(compare_preview(&original, &original).is_ok());
        let mut edited = original.clone();
        edited.title = "different title".into();
        assert!(compare_preview(&edited, &original).is_err());
        let mut edited = original.clone();
        edited.body = "different body".into();
        assert!(compare_preview(&edited, &original).is_err());
        let mut edited = original.clone();
        edited.request.expected_quality_digest = "different quality".into();
        assert!(compare_preview(&edited, &original).is_err());
        let mut edited = original.clone();
        edited.repository = "https://github.com/another/repo.git".into();
        assert!(compare_preview(&edited, &original).is_err());
    }
}
