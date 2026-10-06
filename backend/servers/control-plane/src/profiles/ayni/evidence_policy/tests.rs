use super::*;
use ayni_orchestrator_profile_ayni::managed_runtime::VerifiedRuntime;
use ayni_orchestrator_runtime::coding::{ValidationOutcome, ValidationResult};

#[test]
fn runtime_observation_requires_all_frozen_bindings_but_quality_is_advisory()
-> Result<(), EvidenceError> {
    let expected = RuntimeExpectation {
        lock_fingerprint: "sha256:lock".into(),
        platform: "linux/amd64".into(),
        ayni_version: "1".into(),
    };
    let report = VerificationReport {
        revision: "source".into(),
        lock_digest: "sha256:input".into(),
        check_exit_code: 1,
        validation: ValidationResult {
            outcome: ValidationOutcome::Failed,
            signals_digest: None,
            completed_targets: 0,
            skipped_targets: 0,
            warning_offenders: 0,
            failing_offenders: 0,
            rows: vec![],
        },
        runtime: VerifiedRuntime {
            lock_fingerprint: expected.lock_fingerprint.clone(),
            platform: expected.platform.clone(),
            ayni_version: expected.ayni_version.clone(),
            certificate_key_id: "producer".into(),
            metadata_digest: "sha256:metadata".into(),
            protected_content_root: "sha256:content".into(),
        },
    };
    admit_runtime(&report, &expected, "source", "sha256:input")?;
    for field in 0..6 {
        let mut stale = report.clone();
        match field {
            0 => stale.revision = "other".into(),
            1 => stale.lock_digest = "other".into(),
            2 => stale.runtime.lock_fingerprint = "other".into(),
            3 => stale.runtime.platform = "linux/arm64".into(),
            4 => stale.runtime.ayni_version = "2".into(),
            _ => stale.runtime.certificate_key_id = "unsigned".into(),
        }
        assert_eq!(
            admit_runtime(&stale, &expected, "source", "sha256:input"),
            Err(EvidenceError::BindingMismatch)
        );
    }
    Ok(())
}
#[test]
fn changed_current_policy_requires_unsupported_outcome_while_history_is_retained()
-> Result<(), EvidenceError> {
    assert_eq!(
        admit_candidate_policy(Outcome::Succeeded, true, false),
        Err(EvidenceError::PolicyChanged)
    );
    admit_candidate_policy(Outcome::Unsupported, true, false)?;
    admit_candidate_policy(Outcome::Succeeded, false, false)?;
    admit_candidate_policy(Outcome::Succeeded, true, true)?;
    Ok(())
}
#[test]
fn current_base_drift_takes_precedence_and_history_does_not_borrow_current_base() {
    for provider in [
        ProviderConclusion::Completed,
        ProviderConclusion::Cancelled,
        ProviderConclusion::TimedOut,
    ] {
        assert_eq!(
            observed_outcome(Outcome::Succeeded, provider, true, false),
            Outcome::Stale
        );
    }
    assert_eq!(
        observed_outcome(
            Outcome::Succeeded,
            ProviderConclusion::Completed,
            false,
            false
        ),
        Outcome::Succeeded
    );
    assert_eq!(
        observed_outcome(
            Outcome::Succeeded,
            ProviderConclusion::Cancelled,
            true,
            true
        ),
        Outcome::Cancelled
    );
    assert_eq!(
        observed_outcome(Outcome::Succeeded, ProviderConclusion::TimedOut, true, true),
        Outcome::InfrastructureFailed
    );
}
