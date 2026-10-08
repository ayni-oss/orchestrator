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
fn verification_deadline_bounds_missing_jobs_and_preserves_failure_outcomes() {
    assert_eq!(
        verification_action(&Observation::Running, 1900),
        VerificationAction::Wait
    );
    assert_eq!(
        verification_action(&Observation::Running, 1901),
        VerificationAction::Timeout
    );
    assert_eq!(
        verification_action(&Observation::Missing, 1901),
        VerificationAction::Timeout
    );
    assert_eq!(
        verification_action(&Observation::Missing, 0),
        VerificationAction::Start
    );
    assert_eq!(
        verification_action(&Observation::Failed("failed"), 0),
        VerificationAction::Failure
    );
    assert_eq!(
        verification_action(
            &Observation::Completed {
                digest: "digest".into(),
                report: "report".into()
            },
            1901
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
