//! Shared task and attempt states owned by the control-plane application.
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
