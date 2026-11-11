//! Stage execution for retained Kubernetes runtime preparation operations.
//! Repository programs run only in the unsigned builder. Credentials are supplied
//! individually to a generation-bound stage; they never appear in its evidence.
pub mod client;
pub mod command;
pub mod extraction;
pub mod oci;
pub mod pipeline;
pub mod source;
pub mod supervisor;
pub mod transport;

use ayni_orchestrator_events::execution_diagnostics::{
    CERTIFICATION_DIAGNOSTIC, CertificationDiagnostic,
};
use serde::{Deserialize, Serialize};

/// Closed diagnostics: child output, provider text and credentials are not errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    InvalidInput,
    SourceUnavailable,
    SourceTreeUnavailable,
    SourceFetchUnavailable,
    SourceAccessDenied,
    SourceRateLimited,
    SourceIdentityMismatch,
    SourcePathUnsupported,
    SourceBoundsExceeded,
    ToolUnavailable,
    ExecutionFailed,
    ExecutionDeadline,
    ExecutionOutputExceeded,
    ArtifactInvalid,
    ArtifactConflict,
    ArtifactBoundsExceeded,
    PlatformMismatch,
    RegistryAccessDenied,
    RegistryUnavailable,
    PublicationUncertain,
    CertificationFailed,
    CertificationRejected(CertificationDiagnostic),
    CompatibilityFailed,
    CleanupPending,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.reason())
    }
}
impl std::error::Error for Failure {}
impl Failure {
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::SourceUnavailable => "source_unavailable",
            Self::SourceTreeUnavailable => "source_tree_unavailable",
            Self::SourceFetchUnavailable => "source_fetch_unavailable",
            Self::SourceAccessDenied => "source_access_denied",
            Self::SourceRateLimited => "source_rate_limited",
            Self::SourceIdentityMismatch => "source_identity_mismatch",
            Self::SourcePathUnsupported => "source_path_unsupported",
            Self::SourceBoundsExceeded => "source_bounds_exceeded",
            Self::ToolUnavailable => "tool_unavailable",
            Self::ExecutionFailed => "execution_failed",
            Self::ExecutionDeadline => "execution_deadline",
            Self::ExecutionOutputExceeded => "execution_output_exceeded",
            Self::ArtifactInvalid => "artifact_invalid",
            Self::ArtifactConflict => "artifact_conflict",
            Self::ArtifactBoundsExceeded => "artifact_bounds_exceeded",
            Self::PlatformMismatch => "platform_mismatch",
            Self::RegistryAccessDenied => "registry_access_denied",
            Self::RegistryUnavailable => "registry_unavailable",
            Self::PublicationUncertain => "publication_uncertain",
            Self::CertificationFailed | Self::CertificationRejected(_) => "certification_failed",
            Self::CompatibilityFailed => "compatibility_failed",
            Self::CleanupPending => "cleanup_pending",
        }
    }
}

pub mod telemetry;
