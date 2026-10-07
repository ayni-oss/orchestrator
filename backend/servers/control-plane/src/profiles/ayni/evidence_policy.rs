//! Control-plane authority rules for managed runtime verification.
use ayni_orchestrator_profile_ayni::{
    managed_runtime::RuntimeExpectation, runtime_execution::VerificationReport,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum EvidenceError {
    #[error("managed verification differs from its frozen request")]
    BindingMismatch,
}
impl EvidenceError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::BindingMismatch => "runtime_result_mismatch",
        }
    }
}

/// This checks observation authority; supported Ayni verification remains responsible for trust.
pub fn admit_runtime(
    report: &VerificationReport,
    expected: &RuntimeExpectation,
    source_revision: &str,
    lock_digest: &str,
) -> Result<(), EvidenceError> {
    if report.revision != source_revision
        || report.lock_digest != lock_digest
        || report.runtime.lock_fingerprint != expected.lock_fingerprint
        || report.runtime.platform != expected.platform
        || report.runtime.ayni_version != expected.ayni_version
        || report.runtime.certificate_key_id == "unsigned"
    {
        return Err(EvidenceError::BindingMismatch);
    }
    Ok(())
}
