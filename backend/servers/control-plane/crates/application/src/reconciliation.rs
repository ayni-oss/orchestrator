//! Coding reconciliation decisions independent of execution adapters and wire resources.
use crate::{execution::ExecutionState, tasks::lifecycle::AttemptState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodingAction {
    Start,
    LostExecution,
    WorkerFailed,
    WorkerInterrupted,
    CollectAndCleanup,
    ConfirmRetainedResult,
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
            _ => CodingAction::ConfirmRetainedResult,
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
    fn lost_running_execution_requires_an_explicit_recovery_decision() {
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
            CodingAction::ConfirmRetainedResult
        );
        assert_eq!(
            coding_action(AttemptState::Running, ExecutionState::Running),
            CodingAction::Wait
        );
    }
}

/// Retained worker state resumes the same attempt only while its frozen deadline permits work.
#[must_use]
pub const fn can_resume_worker(
    attempt: AttemptState,
    retained: bool,
    before_deadline: bool,
) -> bool {
    retained && before_deadline && matches!(attempt, AttemptState::Running | AttemptState::Queued)
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn resumption_requires_retained_work_and_an_active_unexpired_attempt() {
        assert!(can_resume_worker(AttemptState::Running, true, true));
        assert!(!can_resume_worker(AttemptState::Running, false, true));
        assert!(!can_resume_worker(AttemptState::Running, true, false));
        for attempt in [
            AttemptState::Cancelled,
            AttemptState::DeadlineExceeded,
            AttemptState::Succeeded,
            AttemptState::Failed,
            AttemptState::WaitingForInput,
            AttemptState::Unsupported,
        ] {
            assert!(!can_resume_worker(attempt, true, true));
        }
    }
}

/// A worker cancellation without a durable task cancellation is an execution disruption.
#[must_use]
pub const fn coding_result_action(
    attempt: AttemptState,
    execution: ExecutionState,
    worker_cancelled: bool,
) -> CodingAction {
    if worker_cancelled
        && matches!(attempt, AttemptState::Queued | AttemptState::Running)
        && matches!(
            execution,
            ExecutionState::Succeeded | ExecutionState::Failed
        )
    {
        CodingAction::WorkerInterrupted
    } else {
        coding_action(attempt, execution)
    }
}

#[cfg(test)]
mod interruption_tests {
    use super::*;
    #[test]
    fn worker_termination_on_completed_or_failed_jobs_preserves_active_task_recovery() {
        for attempt in [AttemptState::Running, AttemptState::Queued] {
            for job in [ExecutionState::Succeeded, ExecutionState::Failed] {
                assert_eq!(
                    coding_result_action(attempt, job, true),
                    CodingAction::WorkerInterrupted
                );
            }
            assert_eq!(
                coding_result_action(attempt, ExecutionState::Running, true),
                CodingAction::Wait
            );
        }
    }
    #[test]
    fn durable_cancellation_and_deadline_override_late_worker_termination() {
        for attempt in [AttemptState::Cancelled, AttemptState::DeadlineExceeded] {
            for job in [
                ExecutionState::Succeeded,
                ExecutionState::Failed,
                ExecutionState::Running,
            ] {
                assert_eq!(
                    coding_result_action(attempt, job, true),
                    CodingAction::Cleanup
                );
                assert!(!can_resume_worker(attempt, true, true));
            }
        }
        assert!(!can_resume_worker(AttemptState::Running, true, false));
        assert_eq!(
            coding_result_action(AttemptState::Running, ExecutionState::Succeeded, false),
            CodingAction::CollectAndCleanup
        );
    }
}
