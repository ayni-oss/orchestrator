//! Publication admission freezes candidate identity and records intent before provider writes.
use crate::tasks::RequestIdentity;
use thiserror::Error;

pub struct PublishCandidate {
    pub identity: RequestIdentity,
    pub candidate_id: String,
}

pub struct CandidateTarget {
    pub candidate_id: String,
    pub candidate_version: u64,
    pub repository_id: String,
    pub base_revision: String,
    pub git_head: Option<String>,
    pub available: bool,
}

pub struct PublicationIntent {
    pub candidate_id: String,
    pub candidate_version: u64,
    pub repository_id: String,
    pub base_revision: String,
    pub head_revision: String,
}

#[derive(Debug, Error)]
pub enum PublicationError<E> {
    #[error("publication persistence failed")]
    Storage(#[source] E),
    #[error("only an available Git candidate can be published")]
    NotPublishable,
}

/// The caller serializes admission; intent, resource and event commit atomically.
pub trait Repository {
    type Error: std::error::Error;

    /// Replay an identical request before reevaluating current candidate state.
    ///
    /// # Errors
    /// Returns a persistence failure or idempotency conflict.
    fn publication_replay(&self, identity: &RequestIdentity)
    -> Result<Option<String>, Self::Error>;

    /// Load immutable candidate identity and current availability.
    ///
    /// # Errors
    /// Returns an error for missing or corrupt resources.
    fn publication_target(&self, candidate_id: &str) -> Result<CandidateTarget, Self::Error>;

    /// Persist the publication resource, provider intent and event together.
    ///
    /// # Errors
    /// Rejects changed candidate versions and atomic persistence failures.
    fn commit_publication(
        &mut self,
        identity: RequestIdentity,
        intent: PublicationIntent,
    ) -> Result<String, Self::Error>;
}

/// Admit a publication intent without performing provider operations.
///
/// # Errors
/// Rejects unavailable/non-Git candidates and returns persistence failures.
pub fn publish<R: Repository>(
    repository: &mut R,
    command: PublishCandidate,
) -> Result<String, PublicationError<R::Error>> {
    if let Some(id) = repository
        .publication_replay(&command.identity)
        .map_err(PublicationError::Storage)?
    {
        return Ok(id);
    }
    let target = repository
        .publication_target(&command.candidate_id)
        .map_err(PublicationError::Storage)?;
    if !target.available {
        return Err(PublicationError::NotPublishable);
    }
    let head = target.git_head.ok_or(PublicationError::NotPublishable)?;
    repository
        .commit_publication(
            command.identity,
            PublicationIntent {
                candidate_id: target.candidate_id,
                candidate_version: target.candidate_version,
                repository_id: target.repository_id,
                base_revision: target.base_revision,
                head_revision: head,
            },
        )
        .map_err(PublicationError::Storage)
}

pub struct AutomaticTarget {
    pub candidate_id: String,
    pub attempt_state: crate::tasks::lifecycle::AttemptState,
    pub task_state: crate::tasks::lifecycle::TaskState,
    pub publication_base_branch: Option<String>,
}

pub struct AutomaticRequest {
    pub key: String,
    pub candidate_id: String,
    pub base_branch: String,
    pub title: String,
    pub body: String,
}

pub trait AutomaticRepository: Repository {
    /// Load a bounded batch of available candidates without existing publications.
    ///
    /// # Errors
    /// Returns failures to load their frozen task and attempt inputs.
    fn automatic_targets(&self) -> Result<Vec<AutomaticTarget>, Self::Error>;

    /// Encode the neutral automatic request using the retained action identity format.
    fn automatic_identity(&self, request: AutomaticRequest) -> RequestIdentity;
}

/// Schedule saved work only when its frozen task requested publication and was not cancelled.
///
/// # Errors
/// Returns publication eligibility, idempotency or persistence failures.
pub fn schedule<R: AutomaticRepository>(
    repository: &mut R,
) -> Result<(), PublicationError<R::Error>> {
    for target in repository
        .automatic_targets()
        .map_err(PublicationError::Storage)?
    {
        let Some(request) = automatic_request(target) else {
            continue;
        };
        let candidate_id = request.candidate_id.clone();
        let identity = repository.automatic_identity(request);
        publish(
            repository,
            PublishCandidate {
                identity,
                candidate_id,
            },
        )?;
    }
    Ok(())
}

fn automatic_request(target: AutomaticTarget) -> Option<AutomaticRequest> {
    use crate::tasks::lifecycle::{AttemptState, TaskState};
    if target.task_state == TaskState::Cancelled
        || !matches!(
            target.attempt_state,
            AttemptState::Succeeded
                | AttemptState::Failed
                | AttemptState::DeadlineExceeded
                | AttemptState::WaitingForInput
        )
    {
        return None;
    }
    Some(AutomaticRequest {
        key: format!("auto-publish:{}", target.candidate_id),
        candidate_id: target.candidate_id,
        base_branch: target.publication_base_branch?,
        title: "feat: implement requested change".into(),
        body: "Requested through the operator application. Review this draft before merging."
            .into(),
    })
}

#[cfg(test)]
mod tests;

pub mod delivery;
