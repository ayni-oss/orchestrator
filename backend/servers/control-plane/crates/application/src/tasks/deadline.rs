//! Deadline eligibility and preservation of already completed attempts.
use super::{
    TimePoint,
    lifecycle::{AttemptState, TaskState},
};
#[must_use]
pub const fn due(deadline: Option<TimePoint>, now: TimePoint) -> bool {
    match deadline {
        Some(value) => {
            value.seconds < now.seconds
                || (value.seconds == now.seconds && value.nanos <= now.nanos)
        }
        None => false,
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Expiration {
    pub expire_attempt: bool,
}
#[must_use]
pub const fn expiration(
    state: TaskState,
    deadline: Option<TimePoint>,
    now: TimePoint,
    attempt: Option<AttemptState>,
) -> Option<Expiration> {
    if !matches!(
        state,
        TaskState::Queued | TaskState::WaitingForRuntime | TaskState::Running
    ) || !due(deadline, now)
    {
        return None;
    }
    Some(Expiration {
        expire_attempt: matches!(attempt, Some(value) if !value.terminal()),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadline_preserves_nanosecond_boundary_and_terminal_attempts() {
        let deadline = TimePoint {
            seconds: 10,
            nanos: 500,
        };
        assert!(!due(
            Some(deadline),
            TimePoint {
                seconds: 10,
                nanos: 499
            }
        ));
        assert!(due(Some(deadline), deadline));
        for attempt in [
            AttemptState::Succeeded,
            AttemptState::Failed,
            AttemptState::Cancelled,
            AttemptState::DeadlineExceeded,
            AttemptState::Unsupported,
        ] {
            assert_eq!(
                expiration(TaskState::Running, Some(deadline), deadline, Some(attempt)),
                Some(Expiration {
                    expire_attempt: false
                })
            );
        }
        assert_eq!(
            expiration(
                TaskState::Running,
                Some(deadline),
                deadline,
                Some(AttemptState::Running)
            ),
            Some(Expiration {
                expire_attempt: true
            })
        );
        assert_eq!(
            expiration(
                TaskState::Cancelled,
                Some(deadline),
                deadline,
                Some(AttemptState::Running)
            ),
            None
        );
    }
}
