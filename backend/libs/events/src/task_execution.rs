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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_paths: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<TaskRecoveryCommand>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRecoveryCommand {
    pub parent_attempt_id: String,
    pub proposal_id: String,
    pub proposal_digest: String,
    pub action: String,
    pub explanation: String,
    pub seed_patch: String,
    pub seed_patch_digest: String,
    pub quality_reason: String,
    pub quality_artifact: String,
    #[serde(default)]
    pub answers: Vec<TaskAnswer>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAnswer {
    pub question_id: String,
    pub question: String,
    pub answer: String,
}
/// A model may request review, but cannot grant itself another execution allowance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRecoveryAdvice {
    pub question: String,
    pub explanation: String,
    pub action: String,
    pub additional_seconds: u32,
    pub additional_tokens: u32,
}
impl TaskRecoveryAdvice {
    #[must_use]
    pub fn validate(&self) -> bool {
        let text = |value: &str| {
            !value.trim().is_empty()
                && value.len() <= 4096
                && value
                    .chars()
                    .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        };
        text(&self.question)
            && text(&self.explanation)
            && matches!(
                self.action.as_str(),
                "continue" | "repair_quality" | "replace_attempt"
            )
            && (1..=86_400).contains(&self.additional_seconds)
            && (1..=1_000_000).contains(&self.additional_tokens)
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_advice: Option<TaskRecoveryAdvice>,
}
/// Prefer accepted coding output; otherwise retain the approved input patch across failed continuations.
#[must_use]
pub fn saved_patch<'a>(command: &'a TaskCommand, outcome: Option<&'a TaskOutcome>) -> &'a str {
    outcome.filter(|o| o.state == "completed").map_or_else(
        || {
            command
                .recovery
                .as_ref()
                .map_or("", |r| r.seed_patch.as_str())
        },
        |o| o.patch.as_str(),
    )
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
                    | "worker_question"
            )
            && self.recovery_advice.as_ref().is_none_or(|advice| {
                self.state == "failed" && self.safe_reason == "worker_question" && advice.validate()
            })
            && (self.safe_reason != "worker_question" || self.recovery_advice.is_some())
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

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn worker_questions_require_bounded_advice_and_cannot_claim_completion() {
        let advice = TaskRecoveryAdvice {
            question: "Which source?".into(),
            explanation: "Clarify the exact reviewed source.".into(),
            action: "continue".into(),
            additional_seconds: 300,
            additional_tokens: 4096,
        };
        let mut outcome = TaskOutcome {
            state: "failed".into(),
            safe_reason: "worker_question".into(),
            recovery_advice: Some(advice),
            ..Default::default()
        };
        assert!(outcome.validate());
        outcome.state = "completed".into();
        assert!(!outcome.validate());
        outcome.state = "failed".into();
        if let Some(advice) = &mut outcome.recovery_advice {
            advice.additional_tokens = 0;
        }
        assert!(!outcome.validate());
        outcome.recovery_advice = None;
        assert!(!outcome.validate());
    }
}

/// Exact reviewed read/write file selection. Empty retains the bounded full-tree policy.
#[must_use]
pub fn valid_source_paths(paths: &[String]) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    paths.len() <= 64
        && paths.iter().all(|path| {
            !path.is_empty()
                && path.len() <= 256
                && !path.chars().any(char::is_control)
                && !matches!(path.as_str(), ".ayni.lock" | ".ayni.toml")
                && !path.contains('\\')
                && path
                    .split('/')
                    .all(|part| !matches!(part, "" | "." | ".." | ".git" | ".ayni"))
                && seen.insert(path)
        })
}

#[cfg(test)]
mod source_selection_tests {
    use super::valid_source_paths;
    #[test]
    fn source_paths_are_exact_bounded_and_exclude_protected_inputs() {
        assert!(valid_source_paths(&[]));
        assert!(valid_source_paths(&[
            "README.md".into(),
            "src/lib.rs".into()
        ]));
        for value in [
            "",
            "/root",
            "../file",
            "a/../file",
            "a//file",
            "./file",
            ".git/config",
            "a/.ayni/ledger",
            ".ayni.lock",
            ".ayni.toml",
            "a\\file",
            "a\nfile",
        ] {
            assert!(!valid_source_paths(&[value.into()]), "{value:?}");
        }
        assert!(!valid_source_paths(&["file".into(), "file".into()]));
        assert!(!valid_source_paths(
            &(0..65).map(|n| format!("file{n}")).collect::<Vec<_>>()
        ));
    }
}
