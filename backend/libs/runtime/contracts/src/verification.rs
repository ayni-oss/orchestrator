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

/// Dedicated Kubernetes `TokenReview` audience; not a Kubernetes API credential.
pub const REPORTING_AUDIENCE: &str = "ayni-orchestrator-verification";
pub const EVENT_FILE: &str = "verification-event.json";
pub const RECEIPT_FILE: &str = "verification-receipt.json";

/// Operator-owned shadow reporting settings.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportingConfiguration {
    pub endpoint: String,
    pub source: String,
}

/// Immutable admission manifest; Pod UID is populated from the downward API.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportingAdmission {
    pub configuration: ReportingConfiguration,
    pub binding: ayni_orchestrator_events::v1::VerificationExecutionBinding,
}

/// Durable receipt is distinct from controller acceptance.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportingReceipt {
    pub source: String,
    pub id: String,
    pub status: String,
}

impl ReportingConfiguration {
    /// Validate bounded operator-owned endpoint/source settings.
    /// # Errors
    /// Rejects unsupported URLs and malformed producer identity.
    pub fn validate(&self) -> Result<(), &'static str> {
        let endpoint = url::Url::parse(&self.endpoint)
            .map_err(|_| "runtime_reporting_configuration_invalid")?;
        if self.endpoint.len() > 512
            || !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/events/verification"
            || !self
                .source
                .strip_prefix("ayni://orchestrator/supervisors/")
                .is_some_and(|id| {
                    !id.is_empty()
                        && id.len() <= 128
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                })
        {
            return Err("runtime_reporting_configuration_invalid");
        }
        Ok(())
    }
}
