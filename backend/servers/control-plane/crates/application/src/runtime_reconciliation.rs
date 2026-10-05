//! Runtime observation and retry decisions without provider, profile or persistence codecs.
use crate::execution::{ExecutionState, Observation};

pub const VERIFICATION_OBSERVATION_TIMEOUT_SECONDS: u64 = 1900;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceIdentity<'a> {
    pub revision: &'a str,
    pub configuration_digest: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingPhase {
    DiscardStale,
    ObserveBuild,
    ObserveVerification,
}

#[must_use]
pub fn pending_phase(
    pending: SourceIdentity<'_>,
    current: SourceIdentity<'_>,
    awaiting_build: bool,
) -> PendingPhase {
    if pending != current {
        return PendingPhase::DiscardStale;
    }
    if awaiting_build {
        PendingPhase::ObserveBuild
    } else {
        PendingPhase::ObserveVerification
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationAction {
    Start,
    Wait,
    Timeout,
    RetainFailedBuildIntent,
    CleanupFailure,
    Accept,
}

#[must_use]
pub const fn verification_action(
    observation: &Observation,
    elapsed_seconds: u64,
    has_build_intent: bool,
) -> VerificationAction {
    match observation {
        Observation::Missing => VerificationAction::Start,
        Observation::Running if elapsed_seconds > VERIFICATION_OBSERVATION_TIMEOUT_SECONDS => {
            VerificationAction::Timeout
        }
        Observation::Running => VerificationAction::Wait,
        Observation::Failed(_) if has_build_intent => VerificationAction::RetainFailedBuildIntent,
        Observation::Failed(_) => VerificationAction::CleanupFailure,
        Observation::Completed { .. } => VerificationAction::Accept,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildAction {
    Start,
    Wait,
    ObserveRegistry,
}

/// An interrupted/failed build may already have published; observing the registry precedes retry.
#[must_use]
pub const fn build_action(state: ExecutionState, started: bool) -> BuildAction {
    match state {
        ExecutionState::Missing if !started => BuildAction::Start,
        ExecutionState::Running => BuildAction::Wait,
        _ => BuildAction::ObserveRegistry,
    }
}

#[must_use]
pub const fn retry_delay(interval: u64, failures: u32) -> u64 {
    let exponent = if failures > 6 { 6 } else { failures };
    let delay = interval.saturating_mul(1_u64 << exponent);
    if delay > 3600 { 3600 } else { delay }
}

#[cfg(test)]
mod tests;
