use super::*;
#[test]
fn changed_source_or_configuration_discards_intent_before_build_or_late_success() {
    let pending = SourceIdentity {
        revision: "a",
        configuration_digest: "one",
    };
    for current in [
        SourceIdentity {
            revision: "b",
            ..pending
        },
        SourceIdentity {
            configuration_digest: "two",
            ..pending
        },
    ] {
        for build in [false, true] {
            assert_eq!(
                pending_phase(pending, current, build),
                PendingPhase::DiscardStale
            );
        }
    }
    assert_eq!(
        pending_phase(pending, pending, true),
        PendingPhase::ObserveBuild
    );
    assert_eq!(
        pending_phase(pending, pending, false),
        PendingPhase::ObserveVerification
    );
}
#[test]
fn uncertain_build_completion_never_automatically_rebuilds() {
    assert_eq!(
        build_action(ExecutionState::Missing, false),
        BuildAction::Start
    );
    for state in [
        ExecutionState::Missing,
        ExecutionState::Failed,
        ExecutionState::Succeeded,
    ] {
        assert_eq!(build_action(state, true), BuildAction::ObserveRegistry);
    }
    assert_eq!(
        build_action(ExecutionState::Running, true),
        BuildAction::Wait
    );
}
#[test]
fn verification_deadline_and_failure_respect_retained_build_intent() {
    assert_eq!(
        verification_action(&Observation::Running, 1900, false),
        VerificationAction::Wait
    );
    assert_eq!(
        verification_action(&Observation::Running, 1901, false),
        VerificationAction::Timeout
    );
    assert_eq!(
        verification_action(&Observation::Missing, 1901, false),
        VerificationAction::Start
    );
    assert_eq!(
        verification_action(&Observation::Failed("failed"), 0, true),
        VerificationAction::RetainFailedBuildIntent
    );
    assert_eq!(
        verification_action(&Observation::Failed("failed"), 0, false),
        VerificationAction::CleanupFailure
    );
    assert_eq!(
        verification_action(
            &Observation::Completed {
                digest: "digest".into(),
                report: "report".into()
            },
            1901,
            false
        ),
        VerificationAction::Accept
    );
}
#[test]
fn retry_backoff_is_bounded_for_repeated_failure_and_large_intervals() {
    assert_eq!(retry_delay(60, 1), 120);
    assert_eq!(retry_delay(60, u32::MAX), 3600);
    assert_eq!(retry_delay(u64::MAX, 1), 3600);
}
