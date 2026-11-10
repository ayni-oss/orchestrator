use super::{Api, Deserialize, Failure, Pod, StageClaim, Supervisor, resources};
use crate::{CERTIFICATION_DIAGNOSTIC, CertificationDiagnostic};
use ayni_orchestrator_events::runtime_preparation::{Evidence, RuntimeProfile, StageReport};
use kube::api::LogParams;
#[derive(Deserialize)]
struct Content {
    state: String,
    policy_digest: String,
    lock_digest: String,
    lock_fingerprint: String,
    certificate_digest: String,
    manifest_digest: String,
    executable_digest: String,
    platform: String,
}
pub(super) async fn finished(
    s: &Supervisor,
    api: &Api<Pod>,
    c: &StageClaim,
    pod: &Pod,
) -> Result<(), Failure> {
    let uid = pod.metadata.uid.as_deref().ok_or(Failure::InvalidInput)?;
    let mut report = StageReport {
        binding: Supervisor::binding(c, uid),
        outcome: "failed".into(),
        safe_reason: "execution_failed".into(),
        evidence: Evidence::default(),
    };
    if c.attempt.stage == "certify" {
        let name = pod.metadata.name.as_deref().ok_or(Failure::InvalidInput)?;
        let logs = api
            .logs(
                name,
                &LogParams {
                    container: Some("stage".into()),
                    limit_bytes: Some(131_072),
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| Failure::ExecutionFailed)?;
        let mut cause = if logs.len() < 131_072
            && let Ok(candidate) = serde_json::from_str::<StageReport>(logs.trim())
            && candidate.binding == report.binding
        {
            report = candidate;
            take_certification_diagnostic(&mut report)
                .unwrap_or(CertificationDiagnostic::WorkerFailureWithoutDiagnostic)
        } else {
            CertificationDiagnostic::MissingOrInvalidWorkerReport
        };
        match resources::verify_signer(s, c).await {
            Ok(()) => {}
            Err(Failure::CertificationFailed) => {
                cause = CertificationDiagnostic::SignerIdentityRejectedAfterExecution;
                report.outcome = "failed".into();
                report.safe_reason = "certification_failed".into();
                report.evidence = Evidence::default();
            }
            Err(reason) => return Err(reason),
        }
        if report.outcome != "completed" {
            let encoded = serde_json::to_string(&cause).map_err(|_| Failure::ExecutionFailed)?;
            report
                .evidence
                .details
                .insert(CERTIFICATION_DIAGNOSTIC.into(), encoded);
        }
        let accepted: ayni_orchestrator_events::runtime_preparation::WorkerResponse = s
            .request("/v1/runtime-supervisor/certification", &report)
            .await?;
        return if accepted.accepted {
            if report.outcome != "completed" {
                tracing::warn!(operation_id = %c.operation.id, generation = c.attempt.generation,
                    pod_uid = uid, cause = ?cause, "Runtime certification failed");
            }
            Ok(())
        } else {
            Err(Failure::ExecutionFailed)
        };
    }
    if matches!(
        c.attempt.stage.as_str(),
        "verify_content" | "verify_compatibility"
    ) {
        let name = pod.metadata.name.as_deref().ok_or(Failure::InvalidInput)?;
        let logs = api
            .logs(
                name,
                &LogParams {
                    container: Some("stage".into()),
                    limit_bytes: Some(65536),
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| Failure::ExecutionFailed)?;
        let result = if logs.len() >= 65536 {
            Err(Failure::ExecutionOutputExceeded)
        } else if c.attempt.stage == "verify_content" {
            content(c, &logs, &mut report)
        } else {
            compatibility(c, &logs, &mut report)
        };
        if let Err(reason) = result {
            report.outcome = "invalid".into();
            report.safe_reason = reason.reason().into();
            report.evidence = Evidence::default();
        }
    }
    s.worker(c, uid)?.report(report).await
}

fn take_certification_diagnostic(report: &mut StageReport) -> Option<CertificationDiagnostic> {
    let encoded = report.evidence.details.remove(CERTIFICATION_DIAGNOSTIC)?;
    serde_json::from_str(&encoded).ok()
}
fn content(c: &StageClaim, logs: &str, report: &mut StageReport) -> Result<(), Failure> {
    let parsed: Content =
        serde_json::from_str(logs.trim()).map_err(|_| Failure::ArtifactInvalid)?;
    let profile: RuntimeProfile = serde_json::from_str(&c.operation.frozen.profile_json)
        .map_err(|_| Failure::InvalidInput)?;
    if parsed.state != "content_verified" {
        report.outcome = "invalid".into();
        report.safe_reason = "artifact_invalid".into();
        return Ok(());
    }
    if parsed.policy_digest != c.operation.frozen.policy_digest
        || parsed.lock_digest != c.operation.frozen.lock_digest
        || parsed.lock_fingerprint != c.operation.frozen.lock_fingerprint
        || parsed.executable_digest != profile.trusted_cli_sha256
        || parsed.platform != profile.platform
        || parsed.certificate_digest.is_empty()
        || parsed.manifest_digest.is_empty()
    {
        return Err(Failure::ArtifactInvalid);
    }
    report.outcome = "completed".into();
    report.safe_reason.clear();
    report.evidence = Evidence {
        content_verified: true,
        certificate_digest: parsed.certificate_digest,
        protected_content_root: parsed.manifest_digest,
        image_digest: c.operation.evidence.image_digest.clone(),
        ..Default::default()
    };
    Ok(())
}
fn compatibility(c: &StageClaim, logs: &str, report: &mut StageReport) -> Result<(), Failure> {
    let value: serde_json::Value =
        serde_json::from_str(logs.trim()).map_err(|_| Failure::CompatibilityFailed)?;
    if value.get("state").and_then(serde_json::Value::as_str) == Some("managed_compatible") {
        report.outcome = "completed".into();
        report.safe_reason.clear();
        let profile: RuntimeProfile = serde_json::from_str(&c.operation.frozen.profile_json)
            .map_err(|_| Failure::InvalidInput)?;
        for (key, expected) in [
            ("source_digest", c.operation.evidence.source_digest.as_str()),
            (
                "lock_fingerprint",
                c.operation.frozen.lock_fingerprint.as_str(),
            ),
            (
                "certificate_digest",
                c.operation.evidence.certificate_digest.as_str(),
            ),
            ("platform", profile.platform.as_str()),
        ] {
            if value.get(key).and_then(serde_json::Value::as_str) != Some(expected) {
                return Err(Failure::CompatibilityFailed);
            }
        }
        let digest = value
            .get("artifact_digest")
            .and_then(serde_json::Value::as_str)
            .ok_or(Failure::CompatibilityFailed)?;
        let hex = digest
            .strip_prefix("sha256:")
            .ok_or(Failure::CompatibilityFailed)?;
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Failure::CompatibilityFailed);
        }
        report.evidence.managed_check_digest = digest.into();
        report.evidence.managed_compatible = true;
    } else {
        report.outcome = "invalid".into();
        report.safe_reason = "compatibility_failed".into();
    }
    Ok(())
}
