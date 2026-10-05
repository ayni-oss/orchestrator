//! Public transport mapping of bounded onboarding failures.
use crate::application::onboarding::{FailureKind, OnboardingError};
use tonic::Status;
pub(super) fn status(error: OnboardingError) -> Status {
    match error.kind {
        FailureKind::InvalidInput => Status::invalid_argument(error.reason()),
        FailureKind::Unavailable => Status::unavailable(error.reason()),
        FailureKind::Missing => Status::not_found(error.reason()),
        FailureKind::Rejected => Status::failed_precondition(error.reason()),
        FailureKind::Internal => Status::internal(error.reason()),
    }
}
pub(super) fn provider_error(reason: &'static str) -> Status {
    status(crate::application::onboarding::provider_error(reason))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn onboarding_failures_preserve_public_codes_and_sanitized_reasons() {
        for (error, code) in [
            (
                OnboardingError::invalid_argument("repository_name_invalid"),
                tonic::Code::InvalidArgument,
            ),
            (
                OnboardingError::unavailable("repository_provider_unconfigured"),
                tonic::Code::Unavailable,
            ),
            (
                OnboardingError::not_found("repository_not_found"),
                tonic::Code::NotFound,
            ),
            (
                OnboardingError::failed_precondition("repository_connection_conflict"),
                tonic::Code::FailedPrecondition,
            ),
            (
                OnboardingError::internal("repository_snapshot_failed"),
                tonic::Code::Internal,
            ),
        ] {
            let response = status(error);
            assert_eq!(response.code(), code);
            assert_eq!(response.message(), error.reason());
        }
        assert_eq!(
            provider_error("github_access_denied").message(),
            "provider_access_required"
        );
        assert_eq!(
            provider_error("github_rate_limited").message(),
            "provider_temporarily_unavailable"
        );
        assert_eq!(
            provider_error("unknown_private_detail").message(),
            "repository_provider_failed"
        );
    }
}
