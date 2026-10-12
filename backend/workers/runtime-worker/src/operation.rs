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
    if bytes.len() as u64 > operations::MAX_RECORD_BYTES {
        return Err(io::Error::other("runtime_report_oversized"));
    }
    let previous = read_bounded(
        &output.join(RECORD_FILE),
        operations::MAX_RECORD_BYTES as usize,
    );
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

pub fn enabled() -> bool {
    Path::new("/request/operation-reporting.json").exists()
}
