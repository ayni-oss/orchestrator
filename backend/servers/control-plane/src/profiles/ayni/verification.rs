//! Bounded retained evidence resolution. No filesystem I/O occurs in a `SQLite` transaction.
use ayni_orchestrator_events::v1::{
    VerificationResultAvailable, verification_result_available::Outcome,
};
use ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport;
use ayni_orchestrator_runtime::verification::{
    CONTRACT, MAX_RECORD_BYTES, RECORD_FILE, VerificationOutcome, VerificationRecord,
};
use std::path::Path;

pub enum Evidence {
    Report(Box<VerificationReport>),
    Failed,
    Invalid(&'static str),
    Unavailable,
}

#[tracing::instrument(
    name = "verification.evidence",
    target = "ayni_orchestrator_observability::verification",
    skip_all
)]
pub fn read(root: &Path, payload: &VerificationResultAvailable) -> Evidence {
    let Some(binding) = &payload.binding else {
        return Evidence::Invalid("binding_missing");
    };
    if !ayni_orchestrator_runtime::verification::valid_execution(&binding.execution_id) {
        return Evidence::Invalid("execution_invalid");
    }
    let reference = match &payload.outcome {
        Some(Outcome::ReportAvailable(report)) => report.evidence.as_ref(),
        Some(Outcome::ExecutionFailed(failure)) => failure.diagnostic.as_ref(),
        None => return Evidence::Invalid("outcome_missing"),
    };
    let Some(reference) = reference else {
        return Evidence::Failed;
    };
    if reference.evidence_id != RECORD_FILE.replace('.', "-") || reference.contract != CONTRACT {
        return Evidence::Invalid("evidence_reference_invalid");
    }
    let path = root.join(&binding.execution_id).join(RECORD_FILE);
    if std::fs::symlink_metadata(&path)
        .is_ok_and(|m| !m.is_file() || m.len() > MAX_RECORD_BYTES as u64)
    {
        return Evidence::Invalid("evidence_unsafe_or_oversized");
    }
    let Ok(bytes) = ayni_orchestrator_runtime::runtime_execution::read_bounded(
        &root.join(&binding.execution_id).join(RECORD_FILE),
        MAX_RECORD_BYTES,
    ) else {
        return Evidence::Unavailable;
    };
    if u64::try_from(bytes.len()).ok() != Some(reference.size_bytes)
        || ayni_orchestrator_domain::digest::sha256(&bytes) != reference.sha256
    {
        return Evidence::Invalid("evidence_digest_mismatch");
    }
    let Ok(record) = serde_json::from_slice::<VerificationRecord<VerificationReport>>(&bytes)
    else {
        return Evidence::Invalid("evidence_invalid");
    };
    if record.version != CONTRACT
        || record.execution != binding.execution_id
        || record.pod_uid != binding.pod_uid
        || record.request_digest != binding.request_digest
    {
        return Evidence::Invalid("evidence_binding_mismatch");
    }
    match (&payload.outcome, record.outcome) {
        (Some(Outcome::ReportAvailable(_)), VerificationOutcome::Report { report }) => {
            Evidence::Report(Box::new(report))
        }
        (Some(Outcome::ExecutionFailed(failure)), VerificationOutcome::Failed { reason })
            if failure_code(reason) == failure.code =>
        {
            Evidence::Failed
        }
        _ => Evidence::Invalid("evidence_outcome_mismatch"),
    }
}

const fn failure_code(reason: ayni_orchestrator_runtime::verification::VerificationFailure) -> i32 {
    use ayni_orchestrator_events::v1::VerificationFailureCode as C;
    use ayni_orchestrator_runtime::verification::VerificationFailure as F;
    (match reason {
        F::RequestInvalid => C::RequestInvalid,
        F::LockMismatch => C::LockMismatch,
        F::PlatformMismatch => C::PlatformMismatch,
        F::VersionUnsupported => C::VersionUnsupported,
        F::CertificateInvalid => C::CertificateInvalid,
        F::EvidenceUnavailable => C::EvidenceUnavailable,
        F::SerializationFailed => C::SerializationFailed,
        F::ReportOversized => C::ReportOversized,
        F::ReportingFailed => C::ReportingFailed,
    }) as i32
}

pub fn failure_reason(code: i32) -> Result<&'static str, &'static str> {
    use ayni_orchestrator_events::v1::VerificationFailureCode as Code;
    Ok(
        match Code::try_from(code).map_err(|_| "runtime_result_invalid")? {
            Code::RequestInvalid => "runtime_request_invalid",
            Code::LockMismatch => "runtime_lock_mismatch",
            Code::PlatformMismatch => "runtime_platform_mismatch",
            Code::VersionUnsupported => "runtime_version_unsupported",
            Code::CertificateInvalid => "runtime_certificate_invalid",
            Code::EvidenceUnavailable => "runtime_evidence_unavailable",
            Code::SerializationFailed => "runtime_report_serialization_failed",
            Code::ReportOversized => "runtime_report_oversized",
            Code::ReportingFailed => "runtime_reporting_failed",
            Code::Unspecified => return Err("runtime_result_invalid"),
        },
    )
}
