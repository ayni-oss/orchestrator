//! Persist-before-send reporting. Repository children cannot access this state
//! or the projected credential. Retry never re-executes repository verification.
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{
        VerificationEvidenceReference, VerificationExecutionFailed, VerificationFailureCode,
        VerificationReportAvailable, VerificationResultAvailable,
        verification_result_available::Outcome,
    },
};
use ayni_orchestrator_runtime::{
    runtime_execution::read_bounded,
    verification::{
        CONTRACT, EVENT_FILE, MAX_RECORD_BYTES, RECEIPT_FILE, RECORD_FILE, ReportingAdmission,
        ReportingReceipt, VerificationFailure, VerificationOutcome, VerificationRecord,
    },
};
use std::{fs, io, path::Path, time::Duration};

fn failed() -> io::Error {
    io::Error::other("runtime_reporting_failed")
}

fn event(output: &Path, admission: &ReportingAdmission, pod_uid: &str) -> io::Result<Vec<u8>> {
    let bytes = read_bounded(&output.join(RECORD_FILE), MAX_RECORD_BYTES)?;
    let record: VerificationRecord<serde_json::Value> =
        serde_json::from_slice(&bytes).map_err(|_| failed())?;
    let mut binding = admission.binding.clone();
    binding.pod_uid = pod_uid.into();
    if record.version != CONTRACT
        || record.execution != binding.execution_id
        || record.pod_uid != pod_uid
        || record.request_digest != binding.request_digest
    {
        return Err(failed());
    }
    let evidence = VerificationEvidenceReference {
        evidence_id: RECORD_FILE.replace('.', "-"),
        sha256: ayni_orchestrator_domain::digest::sha256(&bytes),
        size_bytes: u64::try_from(bytes.len()).map_err(|_| failed())?,
        contract: CONTRACT.into(),
    };
    let outcome = match record.outcome {
        VerificationOutcome::Report { .. } => {
            Outcome::ReportAvailable(VerificationReportAvailable {
                evidence: Some(evidence),
            })
        }
        VerificationOutcome::Failed { reason } => {
            Outcome::ExecutionFailed(VerificationExecutionFailed {
                code: code(reason) as i32,
                diagnostic: Some(evidence),
            })
        }
    };
    let payload = VerificationResultAvailable {
        binding: Some(binding),
        outcome: Some(outcome),
    };
    let path = output.join(EVENT_FILE);
    let previous = match read_bounded(&path, ayni_orchestrator_events::MAX_EVENT_BYTES) {
        Ok(previous) => Some(previous),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(_) => return Err(failed()),
    };
    if let Some(previous) = previous {
        let (identity, retained) =
            ayni_orchestrator_events::decode(&previous).map_err(|_| failed())?;
        if identity.source != admission.configuration.source || retained != payload {
            return Err(failed());
        }
        return Ok(previous);
    }
    let identity = EventIdentity {
        source: admission.configuration.source.clone(),
        id: uuid::Uuid::now_v7().to_string(),
    };
    let bytes = ayni_orchestrator_events::encode(&identity, &payload).map_err(|_| failed())?;
    let bytes = ayni_orchestrator_observability::with_event_context(
        bytes,
        ayni_orchestrator_events::MAX_EVENT_BYTES,
    );
    persist(output, EVENT_FILE, &bytes)?;
    Ok(bytes)
}

const fn code(reason: VerificationFailure) -> VerificationFailureCode {
    match reason {
        VerificationFailure::RequestInvalid => VerificationFailureCode::RequestInvalid,
        VerificationFailure::LockMismatch => VerificationFailureCode::LockMismatch,
        VerificationFailure::PlatformMismatch => VerificationFailureCode::PlatformMismatch,
        VerificationFailure::VersionUnsupported => VerificationFailureCode::VersionUnsupported,
        VerificationFailure::CertificateInvalid => VerificationFailureCode::CertificateInvalid,
        VerificationFailure::EvidenceUnavailable => VerificationFailureCode::EvidenceUnavailable,
        VerificationFailure::SerializationFailed => VerificationFailureCode::SerializationFailed,
        VerificationFailure::ReportOversized => VerificationFailureCode::ReportOversized,
        VerificationFailure::ReportingFailed => VerificationFailureCode::ReportingFailed,
    }
}

pub(crate) fn persist(output: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    use io::Write as _;
    if !fs::symlink_metadata(output)?.is_dir() {
        return Err(failed());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(output)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(output.join(name))
        .map_err(|_| failed())?;
    fs::File::open(output)?.sync_all()
}

/// Deliver a retained observation, replaying its persisted identity until receipt.
/// # Errors
/// Fails closed for invalid retained state, terminal rejection or local persistence failure.
pub async fn deliver(
    output: &Path,
    admission: &ReportingAdmission,
    pod_uid: &str,
    token_path: &Path,
) -> io::Result<()> {
    use tracing::Instrument;
    let context = read_bounded(
        &output.join(EVENT_FILE),
        ayni_orchestrator_events::MAX_EVENT_BYTES,
    )
    .ok()
    .and_then(|bytes| ayni_orchestrator_observability::event_context(&bytes));
    let span = ayni_orchestrator_observability::verification_span(
        "delivery",
        &admission.binding.execution_id,
        context.as_ref(),
    );
    let result = deliver_inner(output, admission, pod_uid, token_path)
        .instrument(span.clone())
        .await;
    if result.is_err() {
        ayni_orchestrator_observability::record_failure(&span, "runtime_reporting_failed");
    }
    result
}

async fn deliver_inner(
    output: &Path,
    admission: &ReportingAdmission,
    pod_uid: &str,
    token_path: &Path,
) -> io::Result<()> {
    admission.configuration.validate().map_err(|_| failed())?;
    let bytes = event(output, admission, pod_uid)?;
    let (identity, _) = ayni_orchestrator_events::decode(&bytes).map_err(|_| failed())?;
    deliver_event(
        output,
        &admission.configuration,
        token_path,
        &identity,
        bytes,
        RECEIPT_FILE,
    )
    .await
}

pub(crate) async fn deliver_event(
    output: &Path,
    configuration: &ayni_orchestrator_runtime::verification::ReportingConfiguration,
    token_path: &Path,
    identity: &EventIdentity,
    bytes: Vec<u8>,
    receipt_file: &str,
) -> io::Result<()> {
    match read_bounded(&output.join(receipt_file), 1024) {
        Ok(receipt) => return validate_receipt(&receipt, &identity),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(failed()),
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| failed())?;
    let mut delay = 1;
    loop {
        // Reopen the projected file for each attempt so token rotation works.
        let token = read_token(token_path)?;
        let result = client
            .post(&configuration.endpoint)
            .bearer_auth(token)
            .header("content-type", "application/cloudevents+json")
            .body(bytes.clone())
            .send()
            .await;
        if let Ok(mut response) = result {
            let status = response.status();
            if status == reqwest::StatusCode::ACCEPTED {
                let mut receipt = Vec::new();
                let mut interrupted = false;
                loop {
                    match response.chunk().await {
                        Ok(Some(chunk)) => {
                            if receipt.len() + chunk.len() > 1024 {
                                return Err(failed());
                            }
                            receipt.extend_from_slice(&chunk);
                        }
                        Ok(None) => break,
                        Err(_) => {
                            interrupted = true;
                            break;
                        }
                    }
                }
                if interrupted {
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    delay = (delay * 2).min(30);
                    continue;
                }
                validate_receipt(&receipt, &identity)?;
                persist(output, receipt_file, &receipt)?;
                return Ok(());
            }
            if !status.is_server_error() && !matches!(status.as_u16(), 408 | 429) {
                return Err(failed());
            }
        }
        tokio::time::sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(30);
    }
}

fn read_token(path: &Path) -> io::Result<String> {
    use io::Read as _;
    // Kubernetes projections deliberately use symlinks; only this trusted path
    // allows them. Repository/evidence readers continue rejecting symlinks.
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(8193).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > 8192 {
        return Err(failed());
    }
    String::from_utf8(bytes)
        .map(|token| token.trim().to_owned())
        .map_err(|_| failed())
}

fn validate_receipt(bytes: &[u8], identity: &EventIdentity) -> io::Result<()> {
    let receipt: ReportingReceipt = serde_json::from_slice(bytes).map_err(|_| failed())?;
    if receipt.source == identity.source && receipt.id == identity.id && receipt.status == "stored"
    {
        Ok(())
    } else {
        Err(failed())
    }
}

#[cfg(test)]
mod tests;
