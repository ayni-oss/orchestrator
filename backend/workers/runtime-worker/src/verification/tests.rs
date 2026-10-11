use super::*;
use ayni_orchestrator_profile_ayni::managed_runtime::VerifiedRuntime;
use ayni_orchestrator_runtime::coding::{QualityOutcome, QualityResult};

fn report() -> VerificationReport {
    VerificationReport {
        revision: "a".repeat(40),
        lock_digest: format!("sha256:{}", "b".repeat(64)),
        runtime: VerifiedRuntime {
            lock_fingerprint: "c".repeat(64),
            platform: "linux/arm64".into(),
            ayni_version: "0.6.3".into(),
            certificate_key_id: "test".into(),
            metadata_digest: format!("sha256:{}", "d".repeat(64)),
            protected_content_root: format!("sha256:{}", "e".repeat(64)),
        },
        check_exit_code: 1,
        quality: QualityResult {
            outcome: QualityOutcome::Failed,
            evidence: None,
            completed_targets: Some(1),
            skipped_targets: Some(0),
            warning_offenders: Some(0),
            failing_offenders: Some(1),
            rows: vec![],
            findings: vec!["finding".repeat(1000)],
        },
    }
}

#[test]
fn large_failed_quality_report_is_committed_before_termination_reference() -> io::Result<()> {
    let output = tempfile::tempdir()?;
    let termination = output.path().join("termination");
    retain(
        output.path(),
        &termination,
        "runtime-test",
        "pod-one",
        "request".into(),
        Ok(report()),
    )?;
    let bytes = read_bounded(&output.path().join(RECORD_FILE), MAX_RECORD_BYTES)?;
    assert!(bytes.len() > 4096);
    let record: VerificationRecord<VerificationReport> = serde_json::from_slice(&bytes)?;
    let VerificationOutcome::Report { report } = record.outcome else {
        return Err(io::Error::other("report missing"));
    };
    assert_eq!(report.check_exit_code, 1);
    assert_eq!(report.quality.outcome, QualityOutcome::Failed);
    assert_eq!(
        fs::read_to_string(termination)?,
        format!("{CONTRACT}:{RECORD_FILE}")
    );
    // A restarted observer reads the same immutable bytes; a second physical writer cannot replace them.
    assert!(
        retain(
            output.path(),
            &output.path().join("termination"),
            "runtime-test",
            "pod-two",
            "request".into(),
            Ok(report)
        )
        .is_err()
    );
    assert_eq!(fs::read(output.path().join(RECORD_FILE))?, bytes);
    Ok(())
}

#[test]
fn termination_delivery_failure_preserves_complete_report() -> io::Result<()> {
    let output = tempfile::tempdir()?;
    let error = retain(
        output.path(),
        &output.path().join("absent/termination"),
        "runtime-test",
        "pod",
        "request".into(),
        Ok(report()),
    )
    .err()
    .ok_or_else(|| io::Error::other("expected failure"))?;
    assert_eq!(error.to_string(), "runtime_reporting_failed");
    let record: VerificationRecord<VerificationReport> =
        serde_json::from_slice(&fs::read(output.path().join(RECORD_FILE))?)?;
    assert!(matches!(record.outcome, VerificationOutcome::Report { .. }));
    Ok(())
}

#[test]
fn failures_retain_only_safe_classification_and_oversized_reports_are_explicit() -> io::Result<()> {
    for oversized in [false, true] {
        let output = tempfile::tempdir()?;
        let result = if oversized {
            let mut report = report();
            report.quality.findings = vec!["x".repeat(MAX_RECORD_BYTES)];
            Ok(report)
        } else {
            Err(classify(&io::Error::other(
                "private command output and credentials",
            )))
        };
        assert!(
            retain(
                output.path(),
                &output.path().join("termination"),
                "runtime-test",
                "pod",
                "request".into(),
                result
            )
            .is_err()
        );
        let bytes = fs::read(output.path().join(RECORD_FILE))?;
        let record: VerificationRecord<VerificationReport> = serde_json::from_slice(&bytes)?;
        let VerificationOutcome::Failed { reason } = record.outcome else {
            return Err(io::Error::other("failure missing"));
        };
        assert_eq!(
            reason.code(),
            if oversized {
                "runtime_report_oversized"
            } else {
                "runtime_evidence_unavailable"
            }
        );
        assert!(!String::from_utf8_lossy(&bytes).contains("private command"));
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn retained_output_rejects_directory_symlinks() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    std::os::unix::fs::symlink(root.path(), root.path().join("link"))?;
    assert!(persist(&root.path().join("link"), b"{}").is_err());
    Ok(())
}
