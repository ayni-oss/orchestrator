//! Canonical, read-only publication review shared by local operator surfaces.
use ayni_orchestrator_api::v1::{
    self, project_conversation_service_client::ProjectConversationServiceClient,
};
use ayni_orchestrator_events::{task_execution::TaskCommand, task_publication};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frozen {
    pub request: v1::StartTaskPublicationRequest,
    pub title: String,
    pub body: String,
    pub repository: String,
    pub source_ref: String,
    pub source_commit: String,
    pub branch: String,
    pub base_branch: String,
    pub author_name: String,
    pub author_email: String,
    pub commit_message: String,
}
/// Read frozen task evidence without authorizing external work.
/// # Errors
/// Rejects missing, incorrectly scoped or ineligible task evidence.
pub async fn review(
    client: &mut ProjectConversationServiceClient<tonic::transport::Channel>,
    selected: v1::GetTaskAttemptRequest,
    key: &str,
) -> Result<Frozen, tonic::Status> {
    let attempt = client
        .get_task_attempt(selected.clone())
        .await?
        .into_inner()
        .attempt
        .ok_or_else(|| tonic::Status::not_found("attempt missing"))?;
    if attempt.state != "completed"
        || attempt.execution_outcome != "completed"
        || attempt.quality.as_ref().is_none_or(|q| q.state != "passed")
        || attempt.patch.is_empty()
    {
        return Err(tonic::Status::failed_precondition(
            "completed changes and passing managed quality required",
        ));
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
        .ok_or_else(|| tonic::Status::not_found("task missing"))?
        .specification
        .ok_or_else(|| tonic::Status::failed_precondition("task specification missing"))?;
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
        repository: task_publication::github_repository(&task.repository).ok_or_else(|| {
            tonic::Status::failed_precondition("unsupported publication repository")
        })?,
        base_branch: task_publication::base_branch(&task.source_ref)
            .ok_or_else(|| tonic::Status::failed_precondition("unsupported publication base"))?,
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
        return Err(tonic::Status::failed_precondition(
            "approved publication metadata exceeds24KiB profile",
        ));
    }
    Ok(frozen)
}
/// Require the reviewed metadata and request to match canonical immutable inputs.
/// # Errors
/// Rejects any edited publication metadata, scope or evidence.
pub fn compare_preview(frozen: &Frozen, canonical: &Frozen) -> Result<(), tonic::Status> {
    if frozen != canonical {
        return Err(tonic::Status::failed_precondition(
            "frozen publication preview differs from immutable reviewed task evidence",
        ));
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
