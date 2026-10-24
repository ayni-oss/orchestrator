use super::{
    ContractError, ConversationTurnBinding, ConversationTurnRequested, EventIdentity,
    MAX_CONTENT_BYTES, digest, resource_identifier,
};

#[allow(
    clippy::suspicious_operation_groupings,
    reason = "snapshot identity binds the event while scope and revision bind its turn"
)]
pub(super) fn validate_project_snapshot(
    payload: &ConversationTurnRequested,
    binding: &ConversationTurnBinding,
    identity: &EventIdentity,
) -> Result<(), ContractError> {
    if let Some(snapshot) = &payload.project_status
        && (snapshot.snapshot_id != identity.id
            || snapshot.project_id != binding.project_id
            || snapshot.context_revision != binding.context_revision
            || snapshot.project_resource_version == 0
            || snapshot.objective.len() > MAX_CONTENT_BYTES
            || snapshot.status != "registered"
            || snapshot.tasks.len() > 20
            || snapshot.total_tasks.map_or(
                !snapshot.tasks.is_empty() || snapshot.has_more_tasks,
                |total| {
                    total.min(20) != snapshot.tasks.len() as u64
                        || snapshot.has_more_tasks != (total > snapshot.tasks.len() as u64)
                },
            )
            || !snapshot.tasks.iter().all(valid_project_task)
            || snapshot.tasks.iter().enumerate().any(|(index, fact)| {
                snapshot.tasks[..index]
                    .iter()
                    .any(|prior| prior.task_id == fact.task_id)
            }))
    {
        return Err(ContractError::Invalid("project status snapshot"));
    }
    Ok(())
}

pub(super) fn valid_project_task(fact: &crate::v1::ProjectTaskFact) -> bool {
    matches!(
        fact.provider_state.as_str(),
        "" | "not_started" | "sending" | "settled" | "uncertain"
    ) && resource_identifier(&fact.task_id)
        && (fact.attempt_id.is_empty() || resource_identifier(&fact.attempt_id))
        && (fact.parent_attempt_id.is_empty()
            || (resource_identifier(&fact.parent_attempt_id)
                && fact.parent_attempt_id != fact.attempt_id
                && !fact.attempt_id.is_empty()))
        && matches!(
            fact.coding_state.as_str(),
            "awaiting_execution"
                | "prepared"
                | "queued"
                | "claimed"
                | "reporting"
                | "cancelling"
                | "completed"
                | "failed"
                | "uncertain"
                | "cancelled"
                | "interrupted"
        )
        && (fact.attempt_id.is_empty() == (fact.coding_state == "awaiting_execution"))
        && matches!(
            fact.quality_state.as_str(),
            "" | "passed" | "failed" | "blocked" | "interrupted"
        )
        && matches!(
            fact.accepted_coding_state.as_str(),
            "" | "completed" | "failed" | "uncertain"
        )
        && (fact.accepted_coding_state.is_empty() || !fact.attempt_id.is_empty())
        && (fact.quality_state.is_empty() || fact.accepted_coding_state == "completed")
        && (fact.quality_artifact_digest.is_empty()
            || (!fact.quality_state.is_empty() && digest(&fact.quality_artifact_digest)))
        && matches!(
            fact.publication_state.as_str(),
            "" | "queued"
                | "claimed"
                | "prepared"
                | "creating"
                | "uncertain"
                | "draft"
                | "blocked"
                | "cancelled"
        )
        && (fact.publication_state.is_empty() == fact.publication_id.is_empty())
        && (fact.publication_id.is_empty()
            || (resource_identifier(&fact.publication_id) && fact.quality_state == "passed"))
        && (fact.pull_request_url.is_empty()
            || (fact.publication_state == "draft"
                && fact.pull_request_url.len() <= 256
                && fact.pull_request_url.starts_with("https://github.com/")
                && fact
                    .pull_request_url
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b":/._-".contains(&byte))))
}
