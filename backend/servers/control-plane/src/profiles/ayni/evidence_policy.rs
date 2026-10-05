//! Control-plane authority rules for managed verification and Actions assessment observations.
use ayni_orchestrator_actions_contract::Outcome;
use ayni_orchestrator_profile_ayni::{
    managed_runtime::RuntimeExpectation, runtime_execution::VerificationReport,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum EvidenceError {
    #[error("managed verification differs from its frozen request")]
    BindingMismatch,
    #[error("candidate policy differs from the retained assessment inputs")]
    PolicyChanged,
}
impl EvidenceError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::BindingMismatch => "runtime_result_mismatch",
            Self::PolicyChanged => "review_candidate_policy_changed",
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

pub fn admit_candidate_policy(
    declared_outcome: Outcome,
    current_candidate: bool,
    identical_inputs: bool,
) -> Result<(), EvidenceError> {
    if current_candidate && !identical_inputs && declared_outcome != Outcome::Unsupported {
        return Err(EvidenceError::PolicyChanged);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderConclusion {
    Completed,
    Cancelled,
    TimedOut,
}

/// Retain historical evidence while preventing a changed current base from carrying authority.
pub const fn observed_outcome(
    declared: Outcome,
    provider: ProviderConclusion,
    current_candidate: bool,
    matching_base: bool,
) -> Outcome {
    if current_candidate && !matching_base {
        return Outcome::Stale;
    }
    match provider {
        ProviderConclusion::Completed => declared,
        ProviderConclusion::Cancelled => Outcome::Cancelled,
        ProviderConclusion::TimedOut => Outcome::InfrastructureFailed,
    }
}

#[cfg(test)]
mod tests;
