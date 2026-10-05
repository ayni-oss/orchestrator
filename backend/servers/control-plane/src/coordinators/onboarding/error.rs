//! Transport-independent onboarding failures with bounded public reasons.
use thiserror::Error;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    InvalidInput,
    Unavailable,
    Missing,
    Rejected,
    Internal,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("{reason}")]
pub struct OnboardingError {
    pub kind: FailureKind,
    reason: &'static str,
}
impl OnboardingError {
    pub const fn reason(self) -> &'static str {
        self.reason
    }
    pub const fn invalid_argument(reason: &'static str) -> Self {
        Self {
            kind: FailureKind::InvalidInput,
            reason,
        }
    }
    pub const fn unavailable(reason: &'static str) -> Self {
        Self {
            kind: FailureKind::Unavailable,
            reason,
        }
    }
    pub const fn not_found(reason: &'static str) -> Self {
        Self {
            kind: FailureKind::Missing,
            reason,
        }
    }
    pub const fn failed_precondition(reason: &'static str) -> Self {
        Self {
            kind: FailureKind::Rejected,
            reason,
        }
    }
    pub const fn internal(reason: &'static str) -> Self {
        Self {
            kind: FailureKind::Internal,
            reason,
        }
    }
}
