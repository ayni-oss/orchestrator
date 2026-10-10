//! Frozen worker inputs and independently reported execution, quality and delivery.
use crate::runtime_execution::ExecutionRequest;
use ayni_orchestrator_domain::ChangeConstraints;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerLimits {
    pub coding_turns: u16,
    pub quality_repair_limit: u8,
    /// Absolute Unix deadline shared by every worker phase.
    pub deadline_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicationDestination {
    pub repository: String,
    pub base_branch: String,
    pub branch: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerSpec<Profile> {
    pub task_id: String,
    pub attempt_id: String,
    pub execution: ExecutionRequest<Profile>,
    pub runtime_digest: String,
    pub request: String,
    pub acceptance_criteria: Vec<String>,
    pub constraints: ChangeConstraints,
    pub model: String,
    pub limits: WorkerLimits,
    pub publication: PublicationDestination,
    pub publication_only: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerPhase {
    Admission,
    Checkout,
    Coding,
    Checking,
    Repairing,
    Finalizing,
    Publishing,
    Completed,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOutcome {
    Succeeded,
    Failed,
    Cancelled,
    DeadlineExceeded,
    AdmissionFailed,
    UnsafeChanges,
    NoChanges,
    InputRequired,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QualityOutcome {
    Passed,
    Failed,
    Unavailable,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PublicationOutcome {
    Pending,
    Publishing,
    Published,
    Failed,
    Conflict,
    NotRequired,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QualityRow {
    pub kind: String,
    pub passed: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub target: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QualityEvidence {
    pub artifact_path: String,
    pub signals_digest: String,
    pub source_fingerprint: String,
    pub commit: String,
}

/// Unknown measurements remain absent. Only supported full-check evidence can pass.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QualityResult {
    pub outcome: QualityOutcome,
    pub evidence: Option<QualityEvidence>,
    pub completed_targets: Option<u32>,
    pub skipped_targets: Option<u32>,
    pub warning_offenders: Option<u32>,
    pub failing_offenders: Option<u32>,
    pub rows: Vec<QualityRow>,
    pub findings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullRequestReceipt {
    pub repository: String,
    pub number: u64,
    pub url: String,
    pub branch: String,
    pub base_branch: String,
    pub head_commit: String,
    pub draft: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerProgress {
    pub phase: WorkerPhase,
    pub coding_turns_consumed: u16,
    pub quality_round: u8,
    pub latest_full_check: Option<QualityResult>,
    pub publication: PublicationOutcome,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerResult {
    pub execution: ExecutionOutcome,
    pub commit: Option<String>,
    pub changed_paths: Vec<String>,
    pub quality: QualityResult,
    pub model_usage: serde_json::Value,
    pub requested_input: Option<String>,
    pub publication: PublicationOutcome,
    pub pull_request: Option<PullRequestReceipt>,
    pub coding_turns_consumed: u16,
    pub quality_round: u8,
}

impl QualityResult {
    /// A pass requires complete measurements and a full artifact bound to a commit.
    #[must_use]
    pub fn is_valid_pass(&self) -> bool {
        self.outcome == QualityOutcome::Passed
            && self.evidence.as_ref().is_some_and(|evidence| {
                !evidence.artifact_path.is_empty()
                    && sha256_digest(&evidence.signals_digest)
                    && sha256_digest(&evidence.source_fingerprint)
                    && git_sha(&evidence.commit)
            })
            && self.completed_targets.is_some_and(|count| count > 0)
            && self.skipped_targets == Some(0)
            && self.warning_offenders.is_some()
            && self.failing_offenders == Some(0)
            && !self.rows.is_empty()
            && self.rows.iter().all(|row| row.passed)
    }
}

fn sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}
fn git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_spec_freezes_attempt_runtime_limits_and_publication() -> Result<(), serde_json::Error>
    {
        #[derive(Clone, Deserialize, Serialize)]
        #[serde(deny_unknown_fields)]
        struct Expectation {
            platform: String,
        }
        let value = serde_json::json!({
            "task_id": "task-1", "attempt_id": "attempt-1",
            "execution": { "repository_url": "https://github.com/team/repository.git",
                "revision": "a".repeat(40), "expected": { "platform": "linux/amd64" } },
            "runtime_digest": format!("sha256:{}", "b".repeat(64)),
            "request": "Implement the change", "acceptance_criteria": ["It works"],
            "constraints": { "protected_paths": ["policy"], "allow_test_edits": false },
            "model": "configured-model",
            "limits": { "coding_turns": 24, "quality_repair_limit": 3, "deadline_at": 1_800_000_000 },
            "publication": { "repository": "team/repository", "base_branch": "main", "branch": "task/attempt-1" },
            "publication_only": false
        });
        let spec: WorkerSpec<Expectation> = serde_json::from_value(value.clone())?;
        assert_eq!(serde_json::to_value(spec)?, value);
        let mut incomplete = value;
        if let Some(object) = incomplete.as_object_mut() {
            object.remove("limits");
        }
        assert!(serde_json::from_value::<WorkerSpec<Expectation>>(incomplete).is_err());
        Ok(())
    }

    #[test]
    fn missing_measurements_and_evidence_cannot_pass() -> Result<(), serde_json::Error> {
        let result = QualityResult {
            outcome: QualityOutcome::Passed,
            evidence: None,
            completed_targets: None,
            skipped_targets: None,
            warning_offenders: None,
            failing_offenders: None,
            rows: vec![],
            findings: vec![],
        };
        assert!(!result.is_valid_pass());
        let value = serde_json::to_value(result)?;
        assert!(value["completed_targets"].is_null());
        Ok(())
    }

    #[test]
    fn full_pass_is_bound_to_immutable_artifact_and_commit() {
        let mut result = QualityResult {
            outcome: QualityOutcome::Passed,
            evidence: Some(QualityEvidence {
                artifact_path: "/output/evidence/signals.json".into(),
                signals_digest: format!("sha256:{}", "a".repeat(64)),
                source_fingerprint: format!("sha256:{}", "b".repeat(64)),
                commit: "c".repeat(40),
            }),
            completed_targets: Some(1),
            skipped_targets: Some(0),
            warning_offenders: Some(0),
            failing_offenders: Some(0),
            rows: vec![QualityRow {
                kind: "test".into(),
                passed: true,
                status: String::new(),
                summary: String::new(),
                target: String::new(),
            }],
            findings: vec![],
        };
        assert!(result.is_valid_pass());
        if let Some(evidence) = result.evidence.as_mut() {
            evidence.signals_digest = "malformed".into();
        }
        assert!(!result.is_valid_pass());
    }
}
