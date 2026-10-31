use super::execution::transitions::{accept_evidence, after_stop};
use super::*;
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
fn operation_fixture(stage: &str) -> Operation {
    Operation {
        id: "operation".into(),
        idempotency_key: "command".into(),
        repository_id: "repository".into(),
        inspection_id: "inspection".into(),
        registry_connection_id: "registry".into(),
        signer_id: "signer".into(),
        platform: "linux/arm64".into(),
        state: "stopping".into(),
        stage: stage.into(),
        safe_reason: String::new(),
        generation: 1,
        allow_build: true,
        allow_replace: false,
        max_builds: 1,
        builds_started: 0,
        deadline_unix_ms: 100,
        accepted_at_unix_ms: 1,
        updated_at_unix_ms: 1,
        frozen_input_digest: digest("frozen"),
        evidence: Evidence::default(),
        frozen: FrozenInputs {
            repository_id: String::new(),
            inspection_id: String::new(),
            source_commit: String::new(),
            source_tree: String::new(),
            source_connection_id: String::new(),
            source_connection_revision: 1,
            source_credential_revision: String::new(),
            installation_id: 1,
            provider_repository_id: 1,
            repository_full_name: String::new(),
            registry_connection_id: String::new(),
            registry_connection_revision: 1,
            registry_credential_revision: String::new(),
            registry_destination: "ghcr.io/ayni-oss/runtime".into(),
            discovery_tag: String::new(),
            reusable_image: String::new(),
            publication_tag: String::new(),
            signer_id: String::new(),
            signer_revision: 1,
            signer_active_revision: String::new(),
            signer_key_id: String::new(),
            signer_public_key: String::new(),
            signer_secret_name: String::new(),
            signer_secret_uid: String::new(),
            policy: String::new(),
            lock: String::new(),
            policy_digest: String::new(),
            lock_digest: String::new(),
            lock_fingerprint: String::new(),
            sources: Vec::new(),
            profile_json: String::new(),
        },
    }
}
fn report(stage: &str, outcome: &str, evidence: Evidence) -> StageReport {
    StageReport {
        binding: Binding {
            operation_id: "operation".into(),
            stage: stage.into(),
            generation: 1,
            pod_uid: "pod".into(),
            command_digest: digest("command"),
        },
        outcome: outcome.into(),
        safe_reason: String::new(),
        evidence,
    }
}
#[test]
fn registry_errors_cannot_authorize_build() -> TestResult {
    let op = operation_fixture("discover");
    for outcome in ["unavailable", "failed", "invalid"] {
        let next = after_stop(
            &op,
            Some(&report("discover", outcome, Evidence::default())),
            "",
        )?;
        assert_eq!(next.stage, "cleanup");
        assert_ne!(next.terminal, "ready");
    }
    assert_eq!(
        after_stop(
            &op,
            Some(&report("discover", "missing", Evidence::default())),
            ""
        )?
        .stage,
        "build"
    );
    Ok(())
}
#[test]
fn replacement_requires_explicit_authority() -> TestResult {
    let mut op = operation_fixture("verify_content");
    let invalid = report("verify_content", "invalid", Evidence::default());
    assert_eq!(after_stop(&op, Some(&invalid), "")?.stage, "cleanup");
    op.allow_replace = true;
    assert_eq!(after_stop(&op, Some(&invalid), "")?.stage, "build");
    Ok(())
}
#[test]
fn readiness_requires_both_verifications_and_cleanup() -> TestResult {
    let mut op = operation_fixture("cleanup");
    let done = report(
        "cleanup",
        "completed",
        Evidence {
            cleanup_complete: true,
            ..Evidence::default()
        },
    );
    assert_ne!(after_stop(&op, Some(&done), "ready")?.state, "ready");
    accept_evidence(&mut op, &done)?;
    op.evidence.image_digest = digest("manifest");
    op.evidence.content_verified = true;
    assert!(after_stop(&op, Some(&done), "ready").is_err());
    op.evidence.managed_compatible = true;
    op.evidence.managed_check_digest = digest("official managed run artifact");
    assert_eq!(after_stop(&op, Some(&done), "ready")?.state, "ready");
    assert_eq!(
        after_stop(&op, Some(&done), "cancelled")?.state,
        "cancelled"
    );
    Ok(())
}
#[test]
fn publish_report_cannot_invent_manifest_intent() -> TestResult {
    let mut op = operation_fixture("publish");
    let manifest = digest("manifest");
    let done = report(
        "publish",
        "completed",
        Evidence {
            image_digest: manifest.clone(),
            ..Evidence::default()
        },
    );
    assert!(accept_evidence(&mut op, &done).is_err());
    op.evidence.expected_manifest_digest = digest("other");
    assert!(accept_evidence(&mut op, &done).is_err());
    op.evidence.expected_manifest_digest = manifest;
    accept_evidence(&mut op, &done)?;
    Ok(())
}
#[test]
fn candidate_freeze_and_certifier_are_different_authorities() -> TestResult {
    let mut op = operation_fixture("freeze_candidate");
    op.evidence.artifact_digest = digest("unsigned");
    let done = report(
        "freeze_candidate",
        "completed",
        Evidence {
            unsigned_manifest_digest: digest("different"),
            artifact_digest: digest("different"),
            protected_content_root: digest("protected"),
            ..Evidence::default()
        },
    );
    assert!(accept_evidence(&mut op, &done).is_err());
    let mut done = done;
    done.evidence.artifact_digest = digest("unsigned");
    done.evidence.unsigned_manifest_digest = digest("unsigned");
    accept_evidence(&mut op, &done)?;
    op.stage = "certify".into();
    let forged = report(
        "certify",
        "completed",
        Evidence {
            certificate_digest: digest("certificate"),
            protected_content_root: digest("new protected"),
            ..Evidence::default()
        },
    );
    assert!(accept_evidence(&mut op, &forged).is_err());
    Ok(())
}
#[test]
fn reports_reject_arbitrary_details_and_artifact_paths() {
    let mut op = operation_fixture("build");
    let mut e = Evidence {
        artifact_digest: digest("layout"),
        artifact_reference: "file:///etc/private".into(),
        ..Evidence::default()
    };
    assert!(accept_evidence(&mut op, &report("build", "completed", e.clone())).is_err());
    e.artifact_reference = format!(
        "runtime://operation/build/1/{}",
        e.artifact_digest.trim_start_matches("sha256:")
    );
    e.details.insert("secret".into(), "value".into());
    assert!(accept_evidence(&mut op, &report("build", "completed", e)).is_err());
}
#[test]
fn immutable_registry_identity_rejects_tags_queries_and_path_traversal() {
    let hash = digest("manifest");
    assert!(validation::image(
        &format!("ghcr.io/owner/image@{hash}"),
        None
    ));
    for repo in [
        "ghcr.io/owner/../image",
        "ghcr.io/owner/image:latest",
        "ghcr.io/owner/image?secret=x",
        "other.example/owner/image",
    ] {
        assert!(!validation::image(&format!("{repo}@{hash}"), None));
    }
    assert!(!validation::image(
        &format!("ghcr.io/owner/image@{hash}"),
        Some("ghcr.io/other/image")
    ));
}

#[test]
fn diagnostic_vocabulary_cannot_carry_arbitrary_provider_or_credential_text() {
    let mut report = report("discover", "unavailable", Evidence::default());
    report.safe_reason = "registry_unavailable".into();
    assert!(validation::report(&report).is_ok());
    for arbitrary in [
        "synthetic_private_pat_value",
        "upstream responded credential=synthetic",
        "https://example.invalid/private",
    ] {
        report.safe_reason = arbitrary.into();
        assert!(validation::report(&report).is_err());
    }
}

#[test]
fn absent_uncertain_publication_cannot_authorize_a_different_build() -> TestResult {
    let mut op = operation_fixture("discover");
    op.evidence.expected_manifest_digest = digest("expected final");
    let next = after_stop(
        &op,
        Some(&report("discover", "missing", Evidence::default())),
        "",
    )?;
    assert_eq!(next.stage, "cleanup");
    assert_eq!(next.terminal, "publication_uncertain");
    Ok(())
}
