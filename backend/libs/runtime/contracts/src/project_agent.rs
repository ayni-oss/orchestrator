//! Immutable input and bounded result for the read-only project research worker.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectAgentRequest {
    pub run_id: String,
    pub project_id: String,
    pub model: String,
    pub history: String,
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
