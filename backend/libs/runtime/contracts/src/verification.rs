//! Retained verification transport: physical identity and bounded, safe outcomes.
use serde::{Deserialize, Serialize};

pub const CONTRACT: &str = "retained-v1";
pub const RECORD_FILE: &str = "verification.json";
pub const MAX_RECORD_BYTES: usize = 131_072;

/// One supervisor-written outcome for a single physical Pod and frozen request.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationRecord<Report> {
    pub version: String,
    pub execution: String,
    pub pod_uid: String,
    pub request_digest: String,
    pub outcome: VerificationOutcome<Report>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum VerificationOutcome<Report> {
    Report { report: Report },
    Failed { reason: VerificationFailure },
}

/// Never retain raw exception text or repository command output as diagnostics.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationFailure {
    RequestInvalid,
    LockMismatch,
    PlatformMismatch,
    VersionUnsupported,
    CertificateInvalid,
    EvidenceUnavailable,
    SerializationFailed,
    ReportOversized,
    ReportingFailed,
}

impl VerificationFailure {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::RequestInvalid => "runtime_request_invalid",
            Self::LockMismatch => "runtime_lock_mismatch",
            Self::PlatformMismatch => "runtime_platform_mismatch",
            Self::VersionUnsupported => "runtime_version_unsupported",
            Self::CertificateInvalid => "runtime_certificate_invalid",
            Self::EvidenceUnavailable => "runtime_evidence_unavailable",
            Self::SerializationFailed => "runtime_report_serialization_failed",
            Self::ReportOversized => "runtime_report_oversized",
            Self::ReportingFailed => "runtime_reporting_failed",
        }
    }
}

#[must_use]
pub fn valid_execution(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
