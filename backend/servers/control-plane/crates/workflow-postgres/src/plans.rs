//! Immutable conversation-owned plan content and explicit operator admission.
use crate::{StoreError, model::Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const MAX_PLAN_BYTES: usize = 32_768;
pub const MAX_PLANS: i64 = 32;
pub const MAX_VERSIONS: i64 = 64;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanContent {
    pub title: String,
    pub objective: String,
    pub scope: String,
    pub out_of_scope: String,
    pub constraints: Vec<String>,
    pub unresolved_questions: Vec<String>,
    pub tasks: Vec<PlanTaskSpecification>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanTaskSpecification {
    pub key: String,
    pub objective: String,
    pub scope: String,
    pub out_of_scope: String,
    pub acceptance_criteria: Vec<String>,
    pub repository: String,
    pub source_ref: String,
    pub execution_limits: String,
    pub unresolved_inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_paths: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanSummary {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub current_version_id: String,
    pub current_ordinal: u64,
    pub title: String,
    pub follows_plan_id: String,
    pub approval_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanVersion {
    pub id: String,
    pub plan_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub ordinal: u64,
    pub predecessor_id: String,
    pub content_digest: String,
    pub content: PlanContent,
    pub author: String,
    pub created_at: Timestamp,
    pub provenance: Option<AgentPlanProvenance>,
}

/// Immutable server-bound source identity; evidence strings are agent declarations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentPlanProvenance {
    pub source_turn_id: String,
    pub configuration_revision_id: String,
    pub context_digest: String,
    pub provider_call_id: String,
    pub provider_response_id: String,
    pub assignment_id: String,
    pub executor_generation: u64,
    pub report_source: String,
    pub report_id: String,
    pub report_digest: String,
    pub invocation_id: String,
    pub assumptions: Vec<String>,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanWriteReceipt {
    pub command_id: String,
    pub plan_id: String,
    pub version_id: String,
    pub conversation_revision: u64,
    pub resume_cursor: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrittenPlan {
    pub version: PlanVersion,
    pub receipt: PlanWriteReceipt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionSummary {
    pub id: String,
    pub ordinal: u64,
    pub content_digest: String,
    pub author: String,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionPage {
    pub versions: Vec<VersionSummary>,
    pub next_before_ordinal: u64,
}

/// A complete immutable owner path, supplied only by the trusted operator adapter.
#[derive(Clone, Copy, Debug)]
pub struct PlanScope<'a> {
    pub workspace: &'a str,
    pub project: &'a str,
    pub conversation: &'a str,
}

fn text(value: &str, max: usize, required: bool) -> Result<(), StoreError> {
    if value.len() > max || value.contains('\0') || (required && value.trim().is_empty()) {
        return Err(StoreError::InvalidInput("plan_content"));
    }
    Ok(())
}

fn items(values: &[String]) -> Result<(), StoreError> {
    if values.len() > 16 {
        return Err(StoreError::Capacity("plan_items"));
    }
    for value in values {
        text(value, 2048, true)?;
    }
    Ok(())
}

/// Validate complete typed content and encode the canonical stored representation.
///
/// # Errors
/// Rejects malformed, duplicate, oversized or unbounded content without truncation.
pub fn canonical_content(content: &PlanContent) -> Result<Vec<u8>, StoreError> {
    text(&content.title, 256, true)?;
    if content.title.trim() != content.title {
        return Err(StoreError::InvalidInput("plan_title"));
    }
    text(&content.objective, 4096, true)?;
    text(&content.scope, 4096, true)?;
    text(&content.out_of_scope, 4096, false)?;
    items(&content.constraints)?;
    items(&content.unresolved_questions)?;
    validate_tasks(&content.tasks)?;
    let bytes = serde_json::to_vec(content).map_err(|_| StoreError::CorruptState)?;
    if bytes.len() > MAX_PLAN_BYTES {
        return Err(StoreError::Capacity("plan_bytes"));
    }
    Ok(bytes)
}

fn validate_tasks(tasks: &[PlanTaskSpecification]) -> Result<(), StoreError> {
    if tasks.len() > 32 {
        return Err(StoreError::Capacity("plan_tasks"));
    }
    let mut keys = BTreeSet::new();
    for task in tasks {
        if task.key.is_empty()
            || task.key.len() > 64
            || !task
                .key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !keys.insert(&task.key)
        {
            return Err(StoreError::InvalidInput("plan_task_key"));
        }
        text(&task.objective, 4096, true)?;
        text(&task.scope, 4096, true)?;
        text(&task.out_of_scope, 4096, false)?;
        text(&task.repository, 1024, false)?;
        text(&task.source_ref, 256, false)?;
        text(&task.execution_limits, 2048, false)?;
        items(&task.acceptance_criteria)?;
        items(&task.unresolved_inputs)?;
        if !ayni_orchestrator_events::task_execution::valid_source_paths(&task.source_paths) {
            return Err(StoreError::InvalidInput("task_source_paths"));
        }
    }
    validate_graph(tasks, &keys)
}

fn validate_graph(
    tasks: &[PlanTaskSpecification],
    keys: &BTreeSet<&String>,
) -> Result<(), StoreError> {
    // Reject missing edges, duplicate edges, self-dependencies and cycles before review.
    let mut resolved = BTreeSet::new();
    for task in tasks {
        let mut dependencies = BTreeSet::new();
        if task.depends_on.len() > 31
            || task
                .depends_on
                .iter()
                .any(|key| key == &task.key || !keys.contains(key) || !dependencies.insert(key))
        {
            return Err(StoreError::InvalidInput("plan_task_dependency"));
        }
    }
    loop {
        let before = resolved.len();
        for task in tasks {
            if task.depends_on.iter().all(|key| resolved.contains(key)) {
                resolved.insert(&task.key);
            }
        }
        if resolved.len() == tasks.len() {
            break;
        }
        if resolved.len() == before {
            return Err(StoreError::InvalidInput("plan_task_dependency"));
        }
    }
    Ok(())
}

pub(crate) fn content_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_dependencies_reject_missing_duplicate_self_and_cyclic_edges() -> Result<(), StoreError>
    {
        let task = |key: &str, dependencies: &[&str]| PlanTaskSpecification {
            key: key.into(),
            objective: "Goal".into(),
            scope: "Scope".into(),
            depends_on: dependencies.iter().map(|key| (*key).into()).collect(),
            ..Default::default()
        };
        let mut content = PlanContent {
            title: "Dependencies".into(),
            objective: "Goal".into(),
            scope: "Scope".into(),
            tasks: vec![
                task("final", &["left", "right"]),
                task("left", &["base"]),
                task("right", &["base"]),
                task("base", &[]),
            ],
            ..Default::default()
        };
        assert!(
            canonical_content(&content).is_ok(),
            "forward references and diamonds are valid"
        );
        for edges in [vec!["missing"], vec!["final"], vec!["left", "left"]] {
            content.tasks[0] = task("final", &edges);
            assert!(matches!(
                canonical_content(&content),
                Err(StoreError::InvalidInput("plan_task_dependency"))
            ));
        }
        content.tasks = vec![task("a", &["b"]), task("b", &["a"])];
        assert!(canonical_content(&content).is_err());
        content.tasks = vec![task("a", &[])];
        let encoded = String::from_utf8(canonical_content(&content)?)
            .map_err(|_| StoreError::CorruptState)?;
        assert!(
            !encoded.contains("depends_on"),
            "empty edges preserve retained canonical content"
        );
        Ok(())
    }
    #[test]
    fn exact_bytes_and_task_keys_are_bounded() -> Result<(), StoreError> {
        let mut value = PlanContent {
            title: "Review".into(),
            objective: "Goal".into(),
            scope: "Scope".into(),
            ..Default::default()
        };
        let bytes = canonical_content(&value)?;
        assert_eq!(canonical_content(&value)?, bytes);
        value.tasks = vec![
            PlanTaskSpecification {
                key: "one".into(),
                objective: "Do it".into(),
                scope: "Scope".into(),
                ..Default::default()
            };
            2
        ];
        assert!(matches!(
            canonical_content(&value),
            Err(StoreError::InvalidInput("plan_task_key"))
        ));
        value.tasks.clear();
        value.constraints = vec!["é".repeat(1024); 16];
        assert!(matches!(
            canonical_content(&value),
            Err(StoreError::Capacity("plan_bytes"))
        ));
        Ok(())
    }
}

/// The retained authorization of one exact immutable plan version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanApprovalReceipt {
    pub id: String,
    pub command_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub plan_id: String,
    pub version_id: String,
    pub content_digest: String,
    pub operator: String,
    pub request_digest: String,
    pub created_at: Timestamp,
    pub conversation_revision: u64,
    pub resume_cursor: String,
    pub task_ids: Vec<String>,
}

/// An admitted specification; source declarations confer no execution authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmittedTask {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub plan_id: String,
    pub version_id: String,
    pub content_digest: String,
    pub approval_id: String,
    pub proposed_task_key: String,
    pub specification: PlanTaskSpecification,
    pub state: String,
    pub execution_blockers: Vec<String>,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovedPlan {
    pub approval: PlanApprovalReceipt,
    pub tasks: Vec<AdmittedTask>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskPage {
    pub tasks: Vec<AdmittedTask>,
    pub next_after_id: String,
}
