//! Bounded, credential-free observations. These records convey no execution authority.
use serde::{Deserialize, Serialize};

pub const SUBJECT: &str = "orchestrator.diagnostics.execution.v1";
pub const MAX_BYTES: usize = 8192;
pub const CERTIFICATION_DIAGNOSTIC: &str = "worker_certification_diagnostic";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Runtime,
    Task,
    Session,
}

impl Kind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Task => "task",
            Self::Session => "session",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub kind: Kind,
    pub execution_id: String,
    pub generation: u64,
    pub stage: String,
    pub event: String,
    pub pod_uid: String,
    pub outcome: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub related_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certification: Option<CertificationDiagnostic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<ContainerDiagnostic>,
}

impl Diagnostic {
    #[must_use]
    pub fn new(kind: Kind, execution_id: &str, generation: u64, stage: &str, event: &str) -> Self {
        Self {
            kind,
            execution_id: execution_id.into(),
            generation,
            stage: stage.into(),
            event: event.into(),
            pod_uid: String::new(),
            outcome: String::new(),
            reason: String::new(),
            related_id: String::new(),
            exit_code: None,
            certification: None,
            containers: Vec::new(),
        }
    }

    /// Supervisor input is a closed metadata projection, never a text log.
    #[must_use]
    pub fn valid_lifecycle(&self) -> bool {
        uuid::Uuid::parse_str(&self.execution_id).is_ok()
            && self.generation > 0
            && (self.pod_uid.is_empty() || uuid::Uuid::parse_str(&self.pod_uid).is_ok())
            && match self.kind {
                Kind::Runtime => crate::runtime_preparation::STAGES.contains(&self.stage.as_str()),
                Kind::Task | Kind::Session => self.stage.is_empty(),
            }
            && matches!(
                self.event.as_str(),
                "pod_lost"
                    | "retiring"
                    | "replacing"
                    | "cancelling"
                    | "finished"
                    | "readiness_timeout"
            )
            && matches!(
                self.outcome.as_str(),
                "pending" | "running" | "succeeded" | "failed" | "unknown"
            )
            && valid_reason(&self.reason)
            && self.certification.is_none()
            && self.related_id.is_empty()
            && self.exit_code.is_none()
            && self.containers.len() <= 16
            && self.containers.iter().all(ContainerDiagnostic::valid)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerDiagnostic {
    pub name: String,
    pub previous: bool,
    pub state: String,
    pub reason: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub restart_count: i32,
}

impl ContainerDiagnostic {
    fn valid(&self) -> bool {
        !self.name.is_empty()
            && self.name.len() <= 63
            && self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && matches!(self.state.as_str(), "terminated" | "waiting")
            && valid_reason(&self.reason)
            && self.restart_count >= 0
    }
}

fn valid_reason(value: &str) -> bool {
    matches!(
        value,
        "" | "completed"
            | "error"
            | "out_of_memory"
            | "deadline_exceeded"
            | "evicted"
            | "node_lost"
            | "container_status_unknown"
            | "container_start_failed"
            | "container_create_failed"
            | "image_pull_failed"
            | "crash_loop"
            | "initializing"
            | "other"
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub cursor: i64,
    pub observed_at_unix_ms: i64,
    pub diagnostic: Diagnostic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificationDiagnostic {
    PolicyInvalid,
    TrustPolicyInvalid,
    LockUnreadable,
    SignerSeedUnreadable,
    SigningMaterialInvalid,
    SignerTrustMismatch,
    PlatformMismatch,
    LockVersionMismatch,
    LockTrustMismatch,
    ProtectedInventoryUnreadable,
    ProtectedInventoryBoundsExceeded,
    ProtectedInventoryRejected,
    ProtectedContentRootInvalid,
    ProtectedContentRootMismatch,
    ProtectedContentChanged,
    ExecutableDigestInvalid,
    ExecutableDigestMismatch,
    LayerWriteFailed,
    ReportWriteFailed,
    CertifierRejected,
    MissingOrInvalidWorkerReport,
    WorkerFailureWithoutDiagnostic,
    SignerIdentityRejectedBeforeExecution,
    SignerIdentityRejectedAfterExecution,
}
