//! Retain one typed terminal outcome before delivering its immutable observation.
use crate::verification::reporting::{deliver_event, persist};
use ayni_orchestrator_events::{
    EventIdentity, operations,
    v1::{RuntimeOperationEvidenceReference, RuntimeOperationResultAvailable},
};
use ayni_orchestrator_runtime::{
    operation::{
        EVENT_FILE, OperationAdmission, OperationOutcome, OperationRecord, RECEIPT_FILE,
        RECORD_FILE,
    },
    runtime_execution::read_bounded,
};
use std::{io, path::Path};

/// Retain and report a terminal operation without rerunning its work on retry.
/// # Errors
/// Rejects conflicts, invalid bindings, oversized evidence and reporting errors.
pub async fn complete(outcome: OperationOutcome) -> io::Result<()> {
    let admission: OperationAdmission = serde_json::from_slice(&read_bounded(
        Path::new("/request/operation-reporting.json"),
        ayni_orchestrator_events::MAX_EVENT_BYTES,
    )?)
    .map_err(io::Error::other)?;
    let uid = std::env::var("POD_UID").map_err(io::Error::other)?;
    report(
        Path::new("/output"),
        &admission,
        &uid,
        Path::new("/reporting/token"),
        outcome,
    )
    .await
}

async fn report(
    output: &Path,
    admission: &OperationAdmission,
    uid: &str,
    token: &Path,
    outcome: OperationOutcome,
) -> io::Result<()> {
    if !std::fs::symlink_metadata(output)?.is_dir() {
        return Err(io::Error::other("runtime_result_invalid"));
    }
    admission
        .configuration
        .validate()
        .map_err(io::Error::other)?;
    if !outcome.matches(admission.kind) {
        return Err(io::Error::other("runtime_result_invalid"));
    }
    let mut binding = admission.binding.clone();
    binding.pod_uid = uid.into();
    let record = OperationRecord {
        contract: operations::CONTRACT.into(),
        binding: binding.clone(),
        outcome,
    };
    let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;
    if bytes.len() > operations::MAX_RECORD_BYTES {
        return Err(io::Error::other("runtime_report_oversized"));
    }
    let previous = read_bounded(&output.join(RECORD_FILE), operations::MAX_RECORD_BYTES);
    match previous {
        Ok(previous) if previous != bytes => {
            return Err(io::Error::other("runtime_result_conflict"));
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => persist(output, RECORD_FILE, &bytes)?,
        Err(e) => return Err(e),
    }
    let payload = RuntimeOperationResultAvailable {
        binding: Some(binding),
        kind: admission.kind as i32,
        evidence: Some(RuntimeOperationEvidenceReference {
            evidence_id: "runtime-operation-json".into(),
            sha256: ayni_orchestrator_domain::digest::sha256(&bytes),
            size_bytes: bytes.len() as u64,
            contract: operations::CONTRACT.into(),
        }),
    };
    let (identity, envelope) = match read_bounded(
        &output.join(EVENT_FILE),
        ayni_orchestrator_events::MAX_EVENT_BYTES,
    ) {
        Ok(bytes) => {
            let (identity, old) = operations::decode(&bytes).map_err(io::Error::other)?;
            if old != payload || identity.source != admission.configuration.source {
                return Err(io::Error::other("runtime_result_conflict"));
            }
            (identity, bytes)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let identity = EventIdentity {
                source: admission.configuration.source.clone(),
                id: uuid::Uuid::now_v7().to_string(),
            };
            let envelope = operations::encode(&identity, &payload).map_err(io::Error::other)?;
            persist(output, EVENT_FILE, &envelope)?;
            (identity, envelope)
        }
        Err(e) => return Err(e),
    };
    deliver_event(
        output,
        &admission.configuration,
        token,
        &identity,
        envelope,
        RECEIPT_FILE,
    )
    .await
}

#[must_use]
pub fn enabled() -> bool {
    Path::new("/request/operation-reporting.json").exists()
}

/// Resume delivery from the retained terminal record without repeating work.
/// # Errors
/// Rejects a changed admission, physical identity or retained result.
pub async fn resume() -> io::Result<()> {
    let admission: OperationAdmission = serde_json::from_slice(&read_bounded(
        Path::new("/request/operation-reporting.json"),
        ayni_orchestrator_events::MAX_EVENT_BYTES,
    )?)
    .map_err(io::Error::other)?;
    let record: OperationRecord = serde_json::from_slice(&read_bounded(
        Path::new("/output").join(RECORD_FILE).as_path(),
        operations::MAX_RECORD_BYTES,
    )?)
    .map_err(io::Error::other)?;
    let uid = std::env::var("POD_UID").map_err(io::Error::other)?;
    let mut binding = admission.binding.clone();
    binding.pod_uid = uid.clone();
    if record.binding != binding || record.contract != operations::CONTRACT {
        return Err(io::Error::other("runtime_result_conflict"));
    }
    report(
        Path::new("/output"),
        &admission,
        &uid,
        Path::new("/reporting/token"),
        record.outcome,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_orchestrator_events::v1::{RuntimeOperationBinding, RuntimeOperationKind};
    #[tokio::test]
    async fn interrupted_delivery_retains_one_identity_and_rejects_changed_result_or_pod()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let digest = format!("sha256:{}", "a".repeat(64));
        let admission = OperationAdmission {
            configuration: ayni_orchestrator_runtime::verification::ReportingConfiguration {
                source: "ayni://orchestrator/supervisors/test".into(),
                endpoint: "http://127.0.0.1:1/events/verification".into(),
            },
            kind: RuntimeOperationKind::Build,
            binding: RuntimeOperationBinding {
                repository_connection_id: "repository".into(),
                runtime_profile_id: "profile".into(),
                runtime_profile_revision_id: "revision".into(),
                preparation_id: "generation".into(),
                execution_id: "build-one".into(),
                pod_uid: String::new(),
                request_digest: digest.clone(),
                source_revision: "a".repeat(40),
                lock_digest: digest.clone(),
                policy_digest: digest.clone(),
                platform: "linux/amd64".into(),
            },
        };
        let token = dir.path().join("missing-token");
        let result = OperationOutcome::Built {
            candidate_image_id: digest,
        };
        assert!(
            report(dir.path(), &admission, "pod-one", &token, result.clone())
                .await
                .is_err()
        );
        let bytes = std::fs::read(dir.path().join(EVENT_FILE))?;
        assert!(
            report(dir.path(), &admission, "pod-one", &token, result.clone())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(dir.path().join(EVENT_FILE))?, bytes);
        let (identity, _) = operations::decode(&bytes)?;
        let receipt = ayni_orchestrator_runtime::verification::ReportingReceipt {
            source: identity.source,
            id: identity.id,
            status: "stored".into(),
        };
        std::fs::write(dir.path().join(RECEIPT_FILE), serde_json::to_vec(&receipt)?)?;
        report(dir.path(), &admission, "pod-one", &token, result.clone()).await?;
        assert!(
            report(dir.path(), &admission, "other-pod", &token, result)
                .await
                .is_err()
        );
        assert!(
            report(
                dir.path(),
                &admission,
                "pod-one",
                &token,
                OperationOutcome::Failed {
                    reason: "runtime_build_failed".into()
                }
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read(dir.path().join(EVENT_FILE))?, bytes);
        let trust_dir = tempfile::tempdir()?;
        let mut trust_admission = admission.clone();
        trust_admission.kind = RuntimeOperationKind::Trust;
        assert!(
            report(
                trust_dir.path(),
                &trust_admission,
                "pod-one",
                &token,
                OperationOutcome::Trust {
                    revision: "a".repeat(40),
                    policy: "p".repeat(5000),
                    lock: "l".repeat(5000),
                },
            )
            .await
            .is_err()
        );
        let large_record = std::fs::read(trust_dir.path().join(RECORD_FILE))?;
        assert!(large_record.len() > 4096);
        let (_, observation) =
            operations::decode(&std::fs::read(trust_dir.path().join(EVENT_FILE))?)?;
        let evidence = observation.evidence.ok_or("evidence")?;
        assert_eq!(evidence.size_bytes, large_record.len() as u64);
        assert_eq!(
            evidence.sha256,
            ayni_orchestrator_domain::digest::sha256(&large_record)
        );
        Ok(())
    }
}
