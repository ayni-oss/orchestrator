//! The supervisor retains verification evidence outside repository-child authority.
use ayni_orchestrator_profile_ayni::runtime_execution::{ExecutionRequest, VerificationReport};
use ayni_orchestrator_runtime::{
    runtime_execution::read_bounded,
    verification::{
        CONTRACT, MAX_RECORD_BYTES, RECORD_FILE, VerificationFailure, VerificationOutcome,
        VerificationRecord, valid_execution,
    },
};
use std::{fs, io, path::Path};

/// Run verification and commit its result before writing a compact Pod reference.
/// # Errors
/// Returns safe classification codes; the retained record survives termination-log failure.
pub async fn run() -> io::Result<()> {
    let execution =
        std::env::var("JOB_NAME").map_err(|_| failure(VerificationFailure::RequestInvalid))?;
    let pod_uid =
        std::env::var("POD_UID").map_err(|_| failure(VerificationFailure::RequestInvalid))?;
    let request_digest = std::env::var("REQUEST_DIGEST")
        .map_err(|_| failure(VerificationFailure::RequestInvalid))?;
    let result = match read_bounded(Path::new("/request/execution.json"), 65_536) {
        Ok(bytes) if ayni_orchestrator_domain::digest::sha256(&bytes) == request_digest => {
            match serde_json::from_slice::<ExecutionRequest>(&bytes) {
                Ok(request) => crate::profiles::ayni::check(&request, Path::new("/workspace"))
                    .await
                    .map_err(|error| classify(&error)),
                Err(_) => Err(VerificationFailure::RequestInvalid),
            }
        }
        _ => Err(VerificationFailure::RequestInvalid),
    };
    retain(
        Path::new("/output"),
        Path::new("/dev/termination-log"),
        &execution,
        &pod_uid,
        request_digest,
        result,
    )
}

fn classify(error: &io::Error) -> VerificationFailure {
    match error.to_string().as_str() {
        "runtime_lock_mismatch" => VerificationFailure::LockMismatch,
        "runtime_platform_mismatch" => VerificationFailure::PlatformMismatch,
        "runtime_version_unsupported" => VerificationFailure::VersionUnsupported,
        "runtime_certificate_invalid" => VerificationFailure::CertificateInvalid,
        _ => VerificationFailure::EvidenceUnavailable,
    }
}

fn failure(reason: VerificationFailure) -> io::Error {
    io::Error::other(reason.code())
}

fn retain(
    output: &Path,
    termination: &Path,
    execution: &str,
    pod_uid: &str,
    request_digest: String,
    result: Result<VerificationReport, VerificationFailure>,
) -> io::Result<()> {
    if !valid_execution(execution) || pod_uid.is_empty() || pod_uid.len() > 128 {
        return Err(failure(VerificationFailure::RequestInvalid));
    }
    let mut record = VerificationRecord {
        version: CONTRACT.into(),
        execution: execution.into(),
        pod_uid: pod_uid.into(),
        request_digest,
        outcome: match result {
            Ok(report) => VerificationOutcome::Report { report },
            Err(reason) => VerificationOutcome::Failed { reason },
        },
    };
    let bytes = match serde_json::to_vec(&record) {
        Ok(bytes) if bytes.len() <= MAX_RECORD_BYTES => bytes,
        result => {
            record.outcome = VerificationOutcome::Failed {
                reason: if result.is_err() {
                    VerificationFailure::SerializationFailed
                } else {
                    VerificationFailure::ReportOversized
                },
            };
            serde_json::to_vec(&record)
                .map_err(|_| failure(VerificationFailure::SerializationFailed))?
        }
    };
    persist(output, &bytes).map_err(|_| failure(VerificationFailure::ReportingFailed))?;
    fs::write(termination, format!("{CONTRACT}:{RECORD_FILE}"))
        .map_err(|_| failure(VerificationFailure::ReportingFailed))?;
    match record.outcome {
        VerificationOutcome::Report { .. } => Ok(()),
        VerificationOutcome::Failed { reason } => Err(failure(reason)),
    }
}

fn persist(output: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    if !fs::symlink_metadata(output)?.is_dir() {
        return Err(io::Error::other("unsafe output directory"));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(output)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    // Never replace a record from another physical execution sharing a Job name.
    temporary
        .persist_noclobber(output.join(RECORD_FILE))
        .map_err(|error| error.error)?;
    fs::File::open(output)?.sync_all()
}

#[cfg(test)]
mod tests;
