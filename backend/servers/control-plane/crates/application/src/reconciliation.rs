//! Coding reconciliation decisions independent of execution adapters and wire resources.
use crate::{execution::ExecutionState, tasks::lifecycle::AttemptState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodingAction {
    Start,
    LostExecution,
    WorkerFailed,
    CollectAndCleanup,
    ConfirmRetainedCandidate,
    Cleanup,
    Wait,
}

/// Terminal interruption cleanup precedes profile dispatch and never starts new work.
#[must_use]
pub const fn coding_action(attempt: AttemptState, execution: ExecutionState) -> CodingAction {
    if matches!(
        attempt,
        AttemptState::Cancelled
            | AttemptState::DeadlineExceeded
            | AttemptState::Failed
            | AttemptState::Unsupported
            | AttemptState::WaitingForInput
    ) {
        return CodingAction::Cleanup;
    }
    match execution {
        ExecutionState::Missing => match attempt {
            AttemptState::Queued => CodingAction::Start,
            AttemptState::Running => CodingAction::LostExecution,
            _ => CodingAction::ConfirmRetainedCandidate,
        },
        ExecutionState::Failed => CodingAction::WorkerFailed,
        ExecutionState::Succeeded => CodingAction::CollectAndCleanup,
        ExecutionState::Running => CodingAction::Wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_running_execution_is_not_resubmitted() {
        assert_eq!(
            coding_action(AttemptState::Running, ExecutionState::Missing),
            CodingAction::LostExecution
        );
        assert_eq!(
            coding_action(AttemptState::Queued, ExecutionState::Missing),
            CodingAction::Start
        );
    }

    #[test]
    fn cancellation_and_terminal_interruptions_override_late_execution_success() {
        for attempt in [
            AttemptState::Cancelled,
            AttemptState::DeadlineExceeded,
            AttemptState::Failed,
            AttemptState::Unsupported,
            AttemptState::WaitingForInput,
        ] {
            for execution in [
                ExecutionState::Missing,
                ExecutionState::Running,
                ExecutionState::Succeeded,
                ExecutionState::Failed,
            ] {
                assert_eq!(coding_action(attempt, execution), CodingAction::Cleanup);
            }
        }
    }

    #[test]
    fn completed_execution_is_collected_before_cleanup() {
        assert_eq!(
            coding_action(AttemptState::Running, ExecutionState::Succeeded),
            CodingAction::CollectAndCleanup
        );
        assert_eq!(
            coding_action(AttemptState::Succeeded, ExecutionState::Missing),
            CodingAction::ConfirmRetainedCandidate
        );
        assert_eq!(
            coding_action(AttemptState::Running, ExecutionState::Running),
            CodingAction::Wait
        );
    }
}
