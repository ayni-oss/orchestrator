//! Cancellation policy independent of transport and persistence representation.
use super::RequestIdentity;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    Queued,
    WaitingForRuntime,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    DeadlineExceeded,
    Unsupported,
}

impl TaskState {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::Cancelled
                | Self::DeadlineExceeded
                | Self::Unsupported
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptState {
    Queued,
    Running,
    WaitingForInput,
    Succeeded,
    Failed,
    Cancelled,
    DeadlineExceeded,
    Unsupported,
}

impl AttemptState {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::Cancelled
                | Self::DeadlineExceeded
                | Self::Unsupported
        )
    }
}

pub struct CancelTask {
    pub identity: RequestIdentity,
    pub task_id: String,
}

pub struct AttemptTarget {
    pub id: String,
    pub state: AttemptState,
    pub version: u64,
}

pub struct CancellationTarget {
    pub task_id: String,
    pub state: TaskState,
    pub version: u64,
    pub attempt: Option<AttemptTarget>,
}

pub struct CancellationPlan {
    pub target: CancellationTarget,
    pub cancel_attempt: bool,
}

#[derive(Debug, Error)]
pub enum CancellationError<E> {
    #[error("task cancellation persistence failed")]
    Storage(#[source] E),
    #[error("a completed task cannot be cancelled")]
    NotActionable,
}

/// Callers serialize decisions and commit each cancellation atomically.
pub trait CancellationRepository {
    type Error: std::error::Error;

    /// Replay the durable receipt for an identical request.
    ///
    /// # Errors
    /// Returns a persistence error or an idempotency conflict.
    fn cancellation_replay(
        &self,
        identity: &RequestIdentity,
    ) -> Result<Option<String>, Self::Error>;

    /// Load the task and active attempt without interpreting cancellation policy.
    ///
    /// # Errors
    /// Returns an error for missing or corrupt resources.
    fn cancellation_target(&self, task_id: &str) -> Result<CancellationTarget, Self::Error>;

    /// Commit the action, selected state changes and events as one transaction.
    /// Already-cancelled tasks only record the new request identity.
    ///
    /// # Errors
    /// Returns an error if resource versions changed or the transaction failed.
    fn commit_cancellation(
        &mut self,
        identity: RequestIdentity,
        plan: CancellationPlan,
    ) -> Result<String, Self::Error>;
}

/// Cancel active work without altering completed attempts or replaying mutations.
///
/// # Errors
/// Rejects completed tasks and returns persistence failures.
pub fn cancel<R: CancellationRepository>(
    repository: &mut R,
    command: CancelTask,
) -> Result<String, CancellationError<R::Error>> {
    if let Some(id) = repository
        .cancellation_replay(&command.identity)
        .map_err(CancellationError::Storage)?
    {
        return Ok(id);
    }
    let target = repository
        .cancellation_target(&command.task_id)
        .map_err(CancellationError::Storage)?;
    if target.state.terminal() && target.state != TaskState::Cancelled {
        return Err(CancellationError::NotActionable);
    }
    let cancel_attempt = target.state != TaskState::Cancelled
        && target
            .attempt
            .as_ref()
            .is_some_and(|attempt| !attempt.state.terminal());
    repository
        .commit_cancellation(
            command.identity,
            CancellationPlan {
                target,
                cancel_attempt,
            },
        )
        .map_err(CancellationError::Storage)
}

#[cfg(test)]
mod tests;
