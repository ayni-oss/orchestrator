//! Bounded task handoff on the authenticated supervisor authority, within v1.
use hmac::{Hmac, Mac as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
pub const MAX_SOURCE_PACK: usize = 8 * 1024 * 1024;
pub const MAX_PATCH: usize = 60 * 1024;
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCommand {
    pub attempt_id: String,
    pub task_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub plan_id: String,
    pub version_id: String,
    pub content_digest: String,
    pub approval_id: String,
    pub configuration_revision_id: String,
    pub generation: u64,
    pub repository: String,
    pub source_commit: String,
    pub source_pack_digest: String,
    pub lock_digest: String,
    pub runtime_image: String,
    pub max_seconds: u32,
    pub max_tokens: u32,
    pub objective: String,
    pub scope: String,
    pub out_of_scope: String,
    pub acceptance_criteria: Vec<String>,
    pub execution_limits: String,
    pub instructions: String,
    pub provider: String,
    pub model: String,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskOutcome {
    pub state: String,
    pub safe_reason: String,
    pub patch: String,
    pub request_digest: String,
    pub response_id: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}
impl TaskOutcome {
    #[must_use]
    pub fn validate(&self) -> bool {
        matches!(self.state.as_str(), "completed" | "failed" | "uncertain")
            && matches!(
                self.safe_reason.as_str(),
                "" | "source_invalid"
                    | "source_context_limit"
                    | "runtime_unavailable"
                    | "provider_disabled"
                    | "provider_uncertain"
                    | "provider_output_invalid"
                    | "execution_interrupted"
                    | "execution_limit"
                    | "cancelled"
            )
            && self.patch.len() <= MAX_PATCH
            && self
                .patch
                .chars()
                .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
            && self.request_digest.len() <= 71
            && self.response_id.len() <= 256
            && (self.state == "completed" || self.patch.is_empty())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskRun {
    pub command: TaskCommand,
    pub state: String,
    pub generation: u64,
    pub pod_uid: String,
    pub deadline_seconds: i64,
    pub provider_state: String,
    pub outcome: Option<TaskOutcome>,
    #[serde(default)]
    pub quality: Option<crate::task_quality::QualityOutcome>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SupervisorRequest {
    Poll,
    Observe {
        attempt_id: String,
        generation: u64,
        pod_uid: String,
    },
    Stopped {
        attempt_id: String,
        generation: u64,
        pod_uid: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerRequest {
    Input,
    Sending {
        request_digest: String,
    },
    Report {
        outcome: TaskOutcome,
    },
    QualityReport {
        outcome: crate::task_quality::QualityOutcome,
    },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkerResponse {
    pub command: Option<TaskCommand>,
    pub source_pack: Vec<u8>,
    pub authorized: bool,
}
#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
/// Derive a scoped bearer; possession cannot mint another attempt/generation token.
#[must_use]
pub fn worker_token(authority: &[u8], attempt: &str, generation: u64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(authority)
        .unwrap_or_else(|_| unreachable!("HMAC accepts every key length"));
    mac.update(format!("ayni-task-worker-v1:{attempt}:{generation}").as_bytes());
    format!("{:x}", mac.finalize().into_bytes())
}
