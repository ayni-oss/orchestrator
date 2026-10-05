//! Typed execution failures preserve stable bounded diagnostic codes at adapter boundaries.
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionErrorKind {
    Configuration,
    InvalidRequest,
    IdentityConflict,
    Unavailable,
    InvalidObservation,
    WorkloadFailed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("{code}")]
pub struct ExecutionError {
    kind: ExecutionErrorKind,
    code: &'static str,
}
impl ExecutionError {
    #[must_use]
    pub const fn kind(self) -> ExecutionErrorKind {
        self.kind
    }
    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }
}
impl From<&'static str> for ExecutionError {
    fn from(code: &'static str) -> Self {
        let kind = match code {
            "runtime_builder_missing"
            | "runtime_configuration_invalid"
            | "runtime_build_configuration_invalid"
            | "kubernetes_incluster_required" => ExecutionErrorKind::Configuration,
            "coding_digest_invalid"
            | "coding_job_invalid"
            | "coding_launcher_invalid"
            | "coding_platform_invalid"
            | "coding_request_invalid"
            | "coding_user_invalid"
            | "runtime_build_invalid"
            | "runtime_request_invalid"
            | "runtime_job_invalid" => ExecutionErrorKind::InvalidRequest,
            "runtime_job_conflict"
            | "runtime_request_conflict"
            | "runtime_pod_identity_invalid" => ExecutionErrorKind::IdentityConflict,
            "kubernetes_unavailable"
            | "runtime_cleanup_failed"
            | "runtime_job_creation_failed"
            | "runtime_source_credential_unavailable" => ExecutionErrorKind::Unavailable,
            "runtime_digest_unavailable" | "runtime_result_invalid" | "runtime_result_missing" => {
                ExecutionErrorKind::InvalidObservation
            }
            "runtime_verification_failed" => ExecutionErrorKind::WorkloadFailed,
            _ => ExecutionErrorKind::Unknown,
        };
        Self { kind, code }
    }
}
impl From<ExecutionError> for &'static str {
    fn from(error: ExecutionError) -> Self {
        error.code
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_conflict_preserves_durable_reason_without_becoming_unavailability() {
        let error = ExecutionError::from("runtime_job_conflict");
        assert_eq!(error.kind(), ExecutionErrorKind::IdentityConflict);
        assert_eq!(<&'static str>::from(error), "runtime_job_conflict");
    }
    #[test]
    fn unknown_adapter_reason_preserves_code_without_assuming_retry_authority() {
        let error = ExecutionError::from("future_reason");
        assert_eq!(error.kind(), ExecutionErrorKind::Unknown);
        assert_eq!(error.code(), "future_reason");
    }
}
