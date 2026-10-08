use super::*;
use serde_json::json;

fn artifact() -> Value {
    json!({
        "invocation": {"command": "check"},
        "source_fingerprint": format!("sha256:{}", "a".repeat(64)),
        "completion": {"scope": "repository", "state": "complete", "completed_targets": 2, "skipped_targets": 0, "issues": []},
        "aggregate": {"status": "pass", "warning_offenders": 0, "failing_offenders": 0},
        "rows": [{"kind": "test", "pass": true, "offenders": {"items": []}, "result": {}}]
    })
}
fn summarize_value(value: &Value, success: bool, full: bool) -> io::Result<QualityResult> {
    summarize(
        &serde_json::to_vec(value)?,
        success,
        full,
        if full { "check" } else { "verify_size" },
        &"b".repeat(40),
    )
}

#[test]
fn focused_checks_cannot_satisfy_full_repository_scope() -> io::Result<()> {
    let full = artifact();
    assert_eq!(
        summarize_value(&full, true, true)?.outcome,
        QualityOutcome::Passed
    );
    assert!(summarize_value(&full, true, false).is_err());
    let mut focused = full;
    focused["invocation"]["command"] = json!("verify_size");
    focused["completion"]["scope"] = json!("requested");
    assert!(summarize_value(&focused, true, true).is_err());
    let evidence = summarize_value(&focused, true, false)?.evidence;
    assert!(evidence.is_none());
    assert!(!summarize_value(&focused, true, false)?.is_valid_pass());
    Ok(())
}

#[test]
fn focused_evidence_matches_the_requested_signal() -> io::Result<()> {
    for (selector, expected) in [
        ("test", "verify_test"),
        ("coverage", "verify_coverage"),
        ("size", "verify_size"),
        ("complexity", "verify_complexity"),
        ("deps", "verify_deps"),
        ("mutation", "verify_mutation"),
    ] {
        let arguments = vec![
            "verify".into(),
            selector.into(),
            "--root".into(),
            ".".into(),
        ];
        let requested = artifact_command(&arguments, false)?;
        assert_eq!(requested, expected);
        let mut value = artifact();
        value["completion"]["scope"] = json!("requested");
        value["invocation"]["command"] = json!(expected);
        let bytes = serde_json::to_vec(&value)?;
        let result = summarize(&bytes, true, false, requested, &"b".repeat(40))?;
        assert!(result.evidence.is_none());
        assert!(!result.is_valid_pass());
        let other = if selector == "size" {
            "verify_test"
        } else {
            "verify_size"
        };
        assert!(summarize(&bytes, true, false, other, &"b".repeat(40)).is_err());
        value["invocation"]["command"] = json!("check");
        assert!(
            summarize(
                &serde_json::to_vec(&value)?,
                true,
                false,
                requested,
                &"b".repeat(40)
            )
            .is_err()
        );
        assert!(artifact_command(&arguments, true).is_err());
    }
    assert!(artifact_command(&["check".into()], false).is_err());
    assert!(artifact_command(&["verify".into(), "list".into()], false).is_err());
    Ok(())
}

#[test]
fn malformed_or_absent_measurements_and_fingerprint_never_pass() {
    for (section, key, replacement) in [
        ("completion", "completed_targets", json!(null)),
        ("completion", "skipped_targets", json!(-1)),
        ("aggregate", "failing_offenders", json!("0")),
        ("aggregate", "warning_offenders", json!(u64::MAX)),
    ] {
        let mut value = artifact();
        value[section][key] = replacement;
        assert!(summarize_value(&value, true, true).is_err());
    }
    for fingerprint in [json!(null), json!("sha256:bad"), json!(0)] {
        let mut value = artifact();
        value["source_fingerprint"] = fingerprint;
        assert!(summarize_value(&value, true, true).is_err());
    }
}

#[test]
fn incomplete_operational_checks_are_unavailable_with_bounded_findings() -> io::Result<()> {
    let mut value = artifact();
    value["completion"]["state"] = json!("incomplete");
    value["completion"]["issues"] = json!(["Runner unavailable"]);
    value["rows"][0]["result"]["failure"] =
        json!({"classification": "timeout", "category": "repo_code_issue", "message": "Timed out"});
    let result = summarize_value(&value, false, true)?;
    assert_eq!(result.outcome, QualityOutcome::Unavailable);
    assert_eq!(result.completed_targets, Some(2));
    assert_eq!(result.findings.len(), 2);
    Ok(())
}

#[test]
fn failed_full_checks_retain_quality_and_actionable_findings() -> io::Result<()> {
    let mut value = artifact();
    value["aggregate"]["status"] = json!("fail");
    value["aggregate"]["failing_offenders"] = json!(1);
    value["rows"][0]["pass"] = json!(false);
    value["rows"][0]["offenders"]["items"] =
        json!([{"path": "src/main.rs", "message": "Failing test"}]);
    let result = summarize_value(&value, false, true)?;
    assert_eq!(result.outcome, QualityOutcome::Failed);
    assert_eq!(result.failing_offenders, Some(1));
    assert!(!result.rows[0].passed);
    assert_eq!(result.findings.len(), 1);
    Ok(())
}

#[test]
fn repository_compile_and_setup_command_failures_allow_repairs() -> io::Result<()> {
    for category in ["repo_code_issue", "repo_setup_issue"] {
        let mut value = artifact();
        value["aggregate"]["status"] = json!("fail");
        value["completion"]["state"] = json!("incomplete");
        value["rows"][0]["pass"] = json!(false);
        value["rows"][0]["result"]["failure"] =
            json!({"classification": "command_error", "category": category, "exit_code": 1});
        assert_eq!(
            summarize_value(&value, false, true)?.outcome,
            QualityOutcome::Failed
        );
    }
    Ok(())
}

#[test]
fn infrastructure_and_measurement_errors_do_not_request_source_repairs() -> io::Result<()> {
    for (category, classification) in [
        ("repo_code_issue", "timeout"),
        ("repo_setup_issue", "cancelled"),
        ("repo_code_issue", "output_limit"),
        ("ayni_internal_issue", "command_error"),
        ("repo_code_issue", "missing_report"),
        ("repo_code_issue", "missing_coverage_metric"),
        ("repo_code_issue", "unparseable_coverage_metric"),
    ] {
        let mut value = artifact();
        value["aggregate"]["status"] = json!("fail");
        value["rows"][0]["result"]["failure"] =
            json!({"classification": classification, "category": category, "exit_code": 1});
        assert_eq!(
            summarize_value(&value, false, true)?.outcome,
            QualityOutcome::Unavailable
        );
    }
    Ok(())
}

#[test]
fn publication_rejects_changed_registry_content_without_rebuild_authority() {
    use ayni_orchestrator_runtime::image_discovery::ImageDiscovery;
    let existing = ImageDiscovery::Resolved {
        digest: format!("sha256:{}", "a".repeat(64)),
        image_id: format!("sha256:{}", "b".repeat(64)),
    };
    assert!(publication_guard(&existing, "different-candidate", false).is_err());
    assert!(publication_guard(&existing, "different-candidate", true).is_ok());
    assert!(publication_guard(&existing, &format!("sha256:{}", "b".repeat(64)), false).is_ok());
    assert!(publication_guard(&ImageDiscovery::Missing, "candidate", false).is_ok());
    assert!(
        publication_guard(
            &ImageDiscovery::Failed {
                reason: "registry_access_denied".into()
            },
            "candidate",
            true
        )
        .is_err()
    );
}
