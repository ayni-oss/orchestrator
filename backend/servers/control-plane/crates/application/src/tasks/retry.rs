//! Explicit retry preserves frozen inputs and commits one new attempt with its replay receipt.
use super::{RequestIdentity, TimePoint, lifecycle::TaskState};
use thiserror::Error;

pub struct RetryTask {
    pub identity: RequestIdentity,
    pub task_id: String,
}
pub struct RetryFacts {
    pub version: u64,
    pub state: TaskState,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub deadline: Option<TimePoint>,
}
#[derive(Debug, Error)]
pub enum RetryError<E> {
    #[error("retry persistence failed")]
    Storage(#[source] E),
    #[error("task is not eligible for retry")]
    NotActionable,
    #[error("the frozen task deadline expired")]
    DeadlineExpired,
}
pub trait Repository {
    type Error: std::error::Error;
    /// # Errors
    /// Returns unavailable/corrupt replay or conflicting identity errors.
    fn replay_retry(&self, identity: &RequestIdentity) -> Result<Option<String>, Self::Error>;
    /// # Errors
    /// Rejects missing/corrupt task and inactive project membership.
    fn retry_facts(&self, task_id: &str) -> Result<RetryFacts, Self::Error>;
    /// # Errors
    /// Returns a clock error when the durable timestamp cannot be represented.
    fn retry_time(&self) -> Result<TimePoint, Self::Error>;
    /// Atomically persist task, new frozen attempt, action receipt and both events.
    /// # Errors
    /// Rejects stale versions and persistence failure without partial events.
    fn commit_retry(
        &mut self,
        command: &RetryTask,
        facts: &RetryFacts,
        now: TimePoint,
    ) -> Result<String, Self::Error>;
}
/// Replay precedes current project, state, count and deadline eligibility.
/// # Errors
/// Returns ineligible/expired task or storage failure.
pub fn retry<R: Repository>(
    repository: &mut R,
    command: &RetryTask,
) -> Result<String, RetryError<R::Error>> {
    if let Some(id) = repository
        .replay_retry(&command.identity)
        .map_err(RetryError::Storage)?
    {
        return Ok(id);
    }
    let facts = repository
        .retry_facts(&command.task_id)
        .map_err(RetryError::Storage)?;
    if !matches!(facts.state, TaskState::Failed | TaskState::DeadlineExceeded)
        || facts.attempt_count >= facts.max_attempts
    {
        return Err(RetryError::NotActionable);
    }
    let now = repository.retry_time().map_err(RetryError::Storage)?;
    if super::deadline::due(facts.deadline, now) {
        return Err(RetryError::DeadlineExpired);
    }
    repository
        .commit_retry(command, &facts, now)
        .map_err(RetryError::Storage)
}
#[cfg(test)]
mod tests;
