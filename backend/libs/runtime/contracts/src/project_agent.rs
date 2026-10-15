//! Immutable input and bounded result for the read-only project research worker.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectAgentRequest {
    pub run_id: String,
    pub project_id: String,
    pub model: String,
    pub history: String,
    /// Frozen, credential-free conversation archive selected for this run.
    #[serde(default)]
    pub conversation_context: serde_json::Value,
    pub repositories: Vec<ResearchRepository>,
    /// Trusted application observations, including runtime readiness and task receipts.
    pub context: String,
    pub max_model_calls: u32,
    pub deadline_seconds: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchRepository {
    pub id: String,
    pub repository_url: String,
    pub revision: String,
    pub coding_connection_id: String,
    pub coding_model: String,
    pub runtime_profile_id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectAgentResult {
    pub run_id: String,
    pub message: String,
    pub proposals: Vec<ResearchProposal>,
    pub model_calls: u32,
    #[serde(default)]
    pub token_usage: ayni_orchestrator_domain::ProviderTokenUsage,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchProposal {
    #[serde(default)]
    pub findings: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub dependency_environment_impact: String,
    pub repository_id: String,
    pub revision: String,
    pub runtime_profile_id: String,
    pub coding_connection_id: String,
    pub coding_model: String,
    pub request: String,
    pub acceptance_criteria: Vec<String>,
    pub allowed_paths: Vec<String>,
    #[serde(default = "allow_test_edits")]
    pub allow_test_edits: bool,
    pub forbidden_paths: Vec<String>,
    pub coding_turns: u32,
    pub deadline_seconds: u64,
    #[serde(default = "create_task")]
    pub action: String,
    #[serde(default)]
    pub target_task_id: String,
}
fn create_task() -> String {
    "create_task".into()
}

/// Map logical identities to bounded filesystem and Kubernetes Secret keys.
#[must_use]
pub fn research_key(id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("repo-{:x}", Sha256::digest(id.as_bytes()))
}

/// Require a direct lifecycle request naming the exact task; quoted or negative
/// discussion of an action cannot grant the model lifecycle authority.
#[must_use]
pub fn explicit_action_request(content: &str, action: &str, task: &str) -> bool {
    if task.is_empty() {
        return false;
    }
    let lower = content.trim().to_ascii_lowercase();
    if action == "retry_publication" && !lower.contains("publication") {
        return false;
    }
    if action == "retry_task" && lower.contains("publication") {
        return false;
    }
    let verb = match action {
        "cancel_task" => "cancel",
        "retry_task" | "retry_publication" => "retry",
        _ => return false,
    };
    let direct = [
        format!("{verb} "),
        format!("please {verb} "),
        format!("can you {verb} "),
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix));
    direct
        && lower.split_whitespace().any(|word| {
            word.trim_matches(|c: char| {
                matches!(c, ',' | '.' | '!' | '?' | '(' | ')' | '[' | ']' | '`')
            }) == task.to_ascii_lowercase()
        })
}
const fn allow_test_edits() -> bool {
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn lifecycle_authority_requires_direct_request_and_exact_task() {
        assert!(super::explicit_action_request(
            "Please cancel task task:123",
            "cancel_task",
            "task:123"
        ));
        assert!(!super::explicit_action_request(
            "Do not cancel task:123",
            "cancel_task",
            "task:123"
        ));
        assert!(!super::explicit_action_request(
            "Investigate why task:123 was cancelled",
            "cancel_task",
            "task:123"
        ));
        assert!(!super::explicit_action_request(
            "Cancel task:1234",
            "cancel_task",
            "task:123"
        ));
    }
}

/// One immutable dispatch into a disposable live planner generation.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSessionTurn {
    pub session_id: String,
    pub generation: u64,
    pub turn_sequence: u64,
    pub pod_uid: String,
    pub deadline_at_unix_seconds: u64,
    pub request: ProjectAgentRequest,
}
/// Source credentials travel only to the trusted checkout container over stdin.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSessionSubmission {
    pub turn: ProjectSessionTurn,
    pub source_tokens: std::collections::BTreeMap<String, String>,
}
/// Local, bounded receipt accepted only against its frozen turn and Pod identity.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSessionReceipt {
    pub session_id: String,
    pub generation: u64,
    pub turn_sequence: u64,
    pub pod_uid: String,
    pub run_id: String,
    pub request_digest: String,
    pub result: Option<ProjectAgentResult>,
    pub failure: String,
}
impl ProjectSessionTurn {
    /// Hash the complete frozen turn independently from refreshed source credentials.
    /// # Errors
    /// Returns an encoding error for an invalid request value.
    pub fn request_digest(&self) -> Result<String, serde_json::Error> {
        use sha2::{Digest, Sha256};
        Ok(format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(self)?)
        ))
    }
    /// Compare every receipt fence without relying on container logs.
    #[must_use]
    pub fn matches_receipt(&self, receipt: &ProjectSessionReceipt) -> bool {
        receipt.session_id == self.session_id
            && receipt.generation == self.generation
            && receipt.turn_sequence == self.turn_sequence
            && receipt.pod_uid == self.pod_uid
            && receipt.run_id == self.request.run_id
            && self
                .request_digest()
                .is_ok_and(|digest| digest == receipt.request_digest)
    }
}
