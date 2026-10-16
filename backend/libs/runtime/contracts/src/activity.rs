//! Bounded worker diagnostics. These records are advisory, never quality evidence.
use serde::{Deserialize, Serialize};

pub const MAX_EVENTS: usize = 256;
pub const MAX_ACTIVITY_BYTES: usize = 196_608;
pub const MAX_DETAIL_BYTES: usize = 8_192;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Request,
    Checkout,
    Build,
    Install,
    RuntimeVerification,
    OutputPreparation,
    ModelCredentials,
    ModelExecution,
    ModelRequest,
    ModelResponse,
    AgentMessage,
    ReasoningSummary,
    ToolStarted,
    ToolFinished,
    Checkpoint,
    Finalization,
    ReportWrite,
    Completed,
}

impl Stage {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Checkout => "Checking out the frozen repository revision",
            Self::Build => "Building the managed runtime",
            Self::Install => "Installing the runtime worker",
            Self::Request => "Reading and validating the frozen request",
            Self::RuntimeVerification => "Verifying the frozen runtime",
            Self::OutputPreparation => "Preparing retained output and usage accounting",
            Self::ModelCredentials => "Loading model credentials and configuring the client",
            Self::ModelExecution => "Running the coding agent",
            Self::ModelRequest => "Waiting for a model response",
            Self::AgentMessage => "Agent message",
            Self::ReasoningSummary => "Provider reasoning summary",
            Self::ModelResponse => "Received a model response",
            Self::ToolStarted => "Running an agent tool",
            Self::ToolFinished => "Agent tool returned",
            Self::Checkpoint => "Saving an atomic recovery generation",
            Self::Finalization => "Checking constraints and binding final work",
            Self::ReportWrite => "Writing the coding result",
            Self::Completed => "Worker completed",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Activity {
    pub sequence: u32,
    pub timestamp_ms: i64,
    pub stage: Stage,
    pub failed: bool,
    #[serde(default)]
    pub failure: Option<Failure>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub details: String,
    #[serde(default)]
    pub truncated: bool,
}

/// Reject malformed, reordered, oversized, or future schema data at the collector boundary.
#[must_use]
pub fn valid(events: &[Activity]) -> bool {
    events.len() <= MAX_EVENTS
        && serde_json::to_vec(events).is_ok_and(|bytes| bytes.len() <= MAX_ACTIVITY_BYTES)
        && events.iter().all(|event| {
            event.sequence > 0 && event.timestamp_ms > 0 && event.details.len() <= MAX_DETAIL_BYTES
        })
        && events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    Authentication,
    RateLimited,
    ProviderUnavailable,
    ProviderRejected,
    Transport,
    InvalidResponse,
    InvalidRequest,
    TurnLimit,
    Deadline,
    InvalidTool,
    Cancelled,
    MissingFile,
    PermissionDenied,
    InvalidInput,
    Internal,
    CommandExit,
    PolicyChanged,
    LockContract,
    LockEvidenceMissing,
    LockEvidenceMismatch,
    DependencyScope,
    NoChanges,
    ProtectedPath,
    TestEdits,
    RuntimeMismatch,
    GitIndex,
    GitStage,
    GitDiff,
    GitCommit,
    GitSource,
    GitIgnored,
    GitIndexLocked,
    GitIdentity,
    GitPermissions,
}
impl Failure {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Authentication => "Model provider rejected authentication or access",
            Self::RateLimited => "Model provider rate or quota limit reached",
            Self::ProviderUnavailable => "Model provider unavailable",
            Self::ProviderRejected => "Model provider rejected the request",
            Self::Transport => "Model transport failed",
            Self::InvalidResponse => "Model response could not be decoded",
            Self::InvalidRequest => "Model request could not be constructed",
            Self::Deadline => "Worker operation reached its time limit",
            Self::TurnLimit => "Agent reached the frozen turn limit",
            Self::InvalidTool => "Agent requested an unavailable tool",
            Self::Cancelled => "Agent run cancelled",
            Self::MissingFile => "Required file or executable is missing",
            Self::PermissionDenied => "Filesystem access denied",
            Self::InvalidInput => "Input is invalid",
            Self::GitIndex => "Git could not reset the worker index",
            Self::GitStage => "Git could not stage worker files",
            Self::GitDiff => "Git could not inspect worker changes",
            Self::GitCommit => "Git could not create the worker commit",
            Self::GitSource => "Git source operation failed",
            Self::GitIgnored => "Git rejected an explicitly staged ignored path",
            Self::GitIndexLocked => "Git index is locked by another operation",
            Self::GitIdentity => "Git commit identity is unavailable",
            Self::GitPermissions => "Git cannot write to the checkout",
            Self::NoChanges => "Work has no changed paths or exceeds path limits",
            Self::ProtectedPath => "Work changes a protected path",
            Self::TestEdits => "Work changes tests without authorization",
            Self::RuntimeMismatch => "Runtime verification detected changed or mismatched source",
            Self::CommandExit => "Command exited unsuccessfully",
            Self::PolicyChanged => "Protected environment policy changed",
            Self::LockContract => {
                "Regenerated environment lock violates the admitted CLI or platform contract"
            }
            Self::LockEvidenceMissing => {
                "Changed environment lock lacks worker regeneration evidence"
            }
            Self::LockEvidenceMismatch => {
                "Environment lock regeneration evidence does not match final dependency inputs"
            }
            Self::DependencyScope => "Dependency change is outside the approved environment scope",
            Self::Internal => "Worker operation failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collector_rejects_arbitrary_content_and_unordered_records() -> Result<(), serde_json::Error>
    {
        assert!(
            serde_json::from_str::<Activity>(
                r#"{"sequence":1,"timestamp_ms":1,"stage":"unknown stage","failed":false}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<Activity>(r#"{"sequence":1,"timestamp_ms":1,"stage":"request","failed":false,"body":"secret"}"#).is_err());
        let event: Activity = serde_json::from_str(
            r#"{"sequence":1,"timestamp_ms":1,"stage":"request","failed":false}"#,
        )?;
        assert!(!valid(&[event.clone(), event.clone()]));
        assert!(!valid(&vec![event; MAX_EVENTS + 1]));
        Ok(())
    }
}
