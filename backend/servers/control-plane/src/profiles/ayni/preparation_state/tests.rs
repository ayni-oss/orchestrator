use super::*;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn persisted_verification_repairs_stale_display_metadata_after_restart() -> TestResult {
    let mut persisted = serde_json::to_value(RuntimeCursor::default())?;
    persisted["status"] = json!({"certificate-trust-state": "pending"});
    persisted["trust"] = json!({
            "generation": "generation", "head": "source", "job": "trust-job",
            "attempt": 0, "result": null, "pull_number": null,
            "state": "verified", "merge_revision": null
    });
    let mut cursor: RuntimeCursor = serde_json::from_value(persisted)?;
    let restarted: RuntimeCursor = serde_json::from_str(&serde_json::to_string(&cursor)?)?;
    let displayed_state = |cursor: &RuntimeCursor| {
        cursor
            .metadata()
            .into_iter()
            .find(|entry| entry.key == "certificate-trust-state")
            .map(|entry| entry.value)
    };
    assert_eq!(displayed_state(&restarted).as_deref(), Some("verified"));
    // Matching certificates alone do not promote an unfinished trust check.
    cursor
        .trust
        .as_mut()
        .ok_or("trust missing")?
        .reset("new-source");
    assert_eq!(displayed_state(&cursor).as_deref(), Some("pending"));
    cursor.trust = None;
    assert_eq!(displayed_state(&cursor).as_deref(), Some("pending"));
    Ok(())
}

pub fn legacy_cursor() -> Value {
    json!({
        "sequence": 4, "next_at": 60, "last_success": null, "failures": 1,
        "bindings": {}, "status": {"builder-contract":"generation", "builder-image":"ghcr.io/ayni-oss/ayni-builder@sha256:abc", "builder-job":"builder-frozen", "builder-started":"21"},
        "pending": {
            "name":"verify-frozen", "profile_revision":"revision-frozen",
            "request":{"repository_url":"https://github.com/ayni-oss/sandbox.git","revision":"source-frozen", "expected":{"lock_fingerprint":"sha256:abc","platform":"linux/arm64","ayni_version":"0.15.0"}},
            "image":"ghcr.io/ayni-oss/sandbox:runtime", "frozen_runtime": {
                "image":"ghcr.io/ayni-oss/sandbox:runtime", "platform":"linux/arm64", "service_account":"runtime-coding",
                "pull_secret":"retired-pull", "build":{"registry_secret":"retired-push", "signing_secret":"retired-signing"}
            },
            "started_at":21,"source_digest":"source-digest","lock_digest":"lock-digest",
            "build_name":null,"build_started":false,"build_completed":false,"build_observed":false,
            "discovery_started":false,"discovery_complete":false,
            "publish_attempts":2,"publish_remaining":1,
            "certificate":{"id":"certificate-frozen","revision":"2","key_id":"key","public_key":"public","secret_name":"retired-signing","enabled":true,"status":"ready"}
        }
    })
}

#[test]
fn every_active_legacy_image_phase_preserves_identity_and_budget_on_upgrade() -> TestResult {
    for (phase, started, completed, observed, discovery_started, discovery_complete) in [
        (
            ImagePreparationPhase::Discovering,
            false,
            false,
            false,
            false,
            false,
        ),
        (
            ImagePreparationPhase::DiscoveringStarted,
            false,
            false,
            false,
            true,
            false,
        ),
        (
            ImagePreparationPhase::Building,
            false,
            false,
            false,
            true,
            true,
        ),
        (
            ImagePreparationPhase::BuildingStarted,
            true,
            false,
            false,
            true,
            true,
        ),
        (
            ImagePreparationPhase::Publishing,
            true,
            true,
            false,
            true,
            true,
        ),
        (
            ImagePreparationPhase::PublicationReadback,
            true,
            true,
            true,
            false,
            false,
        ),
        (
            ImagePreparationPhase::PublicationReadbackStarted,
            true,
            true,
            true,
            true,
            false,
        ),
        (
            ImagePreparationPhase::Verifying,
            true,
            true,
            true,
            true,
            true,
        ),
    ] {
        let mut value = legacy_cursor();
        let pending = &mut value["pending"];
        pending["build_started"] = started.into();
        pending["build_completed"] = completed.into();
        pending["build_observed"] = observed.into();
        pending["discovery_started"] = discovery_started.into();
        pending["discovery_complete"] = discovery_complete.into();
        if discovery_complete || completed {
            pending["build_name"] = "build-frozen".into();
        }
        if completed {
            pending["candidate_image_id"] = "candidate-frozen".into();
        }
        if observed {
            pending["publish_name"] = "publisher-frozen".into();
        }
        if phase == ImagePreparationPhase::Verifying {
            pending["resolved_digest"] = "sha256:resolved".into();
        }
        let cursor: RuntimeCursor = serde_json::from_value(value)?;
        assert_eq!(
            cursor.pending.as_ref().ok_or("pending missing")?.phase,
            phase
        );
        assert_eq!(cursor.builder.started_at, Some(21));
        assert!(!cursor.status.contains_key("builder-job"));
        let restarted = roundtrip(&cursor)?;
        let pending = restarted.pending.ok_or("pending missing")?;
        assert_eq!(pending.name, "verify-frozen");
        assert_eq!(pending.request.revision, "source-frozen");
        assert_eq!(pending.publish_attempts, 2);
        assert_eq!(pending.publish_remaining, 1);
    }
    Ok(())
}

#[test]
fn preparation_retains_credentials_before_jobs_and_releases_them_after_completion() -> TestResult {
    let mut cursor: RuntimeCursor = serde_json::from_value(legacy_cursor())?;
    let pending = cursor.pending.take().ok_or("pending missing")?;
    cursor.preparation = Some(PreparationIdentity {
        operation_reporting: None,
        operation_observations: std::collections::BTreeMap::new(),
        verification_authority:
            crate::profiles::ayni::preparation_state::VerificationAuthority::Retained,
        generation: "frozen".into(),
        source_revision: pending.request.revision,
        policy_digest: "policy".into(),
        lock_digest: pending.lock_digest,
        contract_version: "0.15.0".into(),
        platform: "linux/arm64".into(),
        registry: vec![],
        certificate: pending.certificate,
        runtime: pending.frozen_runtime.ok_or("runtime missing")?,
    });
    let refs = cursor.credential_references();
    for name in ["retired-pull", "retired-push", "retired-signing"] {
        assert!(refs.secrets.contains(name));
    }
    assert!(refs.certificates.contains("certificate-frozen"));
    cursor.preparation = None;
    assert_eq!(
        cursor.credential_references(),
        CredentialReferences::default()
    );
    Ok(())
}

#[test]
fn corrupt_or_unknown_preparation_state_fails_closed() {
    let mut cursor = legacy_cursor();
    cursor["pending"]["phase"] = "future_phase".into();
    assert!(serde_json::from_value::<RuntimeCursor>(cursor).is_err());
    let mut cursor = legacy_cursor();
    cursor["pending"]["phase"] = "publishing".into();
    assert!(serde_json::from_value::<RuntimeCursor>(cursor).is_err());
    let mut cursor = legacy_cursor();
    cursor["status"]["builder-started"] = "invalid".into();
    assert!(serde_json::from_value::<RuntimeCursor>(cursor).is_err());
}

fn roundtrip(cursor: &RuntimeCursor) -> Result<RuntimeCursor, serde_json::Error> {
    let serialized = serde_json::to_value(cursor)?;
    assert!(serialized["pending"].get("build_started").is_none());
    let restarted: RuntimeCursor = serde_json::from_value(serialized.clone())?;
    assert_eq!(serde_json::to_value(&restarted)?, serialized);
    Ok(restarted)
}

#[test]
fn retained_verification_survives_upgrade_during_cleanup_and_failure_retry() -> TestResult {
    let mut value = legacy_cursor();
    let report = json!({
        "revision":"source-frozen", "lock_digest":"lock-digest",
        "runtime":{"lock_fingerprint":"sha256:abc","platform":"linux/arm64",
            "ayni_version":"0.15.0","certificate_key_id":"key",
            "metadata_digest":"sha256:metadata","protected_content_root":"sha256:content"},
        "check_exit_code":0,"quality":{"outcome":"unavailable","evidence":null,
            "completed_targets":null,"skipped_targets":null,"warning_offenders":null,
            "failing_offenders":null,"rows":[],"findings":[]}
    });
    value["pending"]["verification"] = report.clone();
    value["pending"]["resolved_digest"] = "sha256:resolved".into();
    value["pending"]["failed"] = true.into();
    let cursor: RuntimeCursor = serde_json::from_value(value)?;
    let mut restarted = roundtrip(&cursor)?;
    assert_eq!(
        restarted.pending.as_ref().ok_or("pending missing")?.phase,
        ImagePreparationPhase::Activating
    );
    restarted.retry();
    let pending = restarted.pending.ok_or("pending missing")?;
    assert!(!pending.failed);
    assert_eq!(pending.phase, ImagePreparationPhase::Activating);
    assert_eq!(serde_json::to_value(pending.verification)?, report);
    assert_eq!(pending.publish_remaining, 1);
    Ok(())
}

#[test]
fn legacy_observation_counts_never_become_recorded_execution_history() -> TestResult {
    let mut value = legacy_cursor();
    value["failures"] = 3.into();
    let mut cursor: RuntimeCursor = serde_json::from_value(value)?;
    assert_eq!(cursor.failures, 3);
    assert_eq!(cursor.accounting.legacy_failures, 3);
    assert!(cursor.accounting.executions.is_empty());
    assert!(cursor.retry_exhausted());
    assert!(
        !cursor
            .metadata()
            .iter()
            .any(|entry| entry.key == "next-retry")
    );
    cursor.retry();
    let restarted = roundtrip(&cursor)?;
    assert_eq!(restarted.failures, 0);
    assert_eq!(restarted.accounting.legacy_failures, 3);
    assert!(restarted.accounting.executions.is_empty());
    Ok(())
}
