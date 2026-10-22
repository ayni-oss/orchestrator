//! Immutable operator-authored plan content. No execution or approval authority.
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
    if content.tasks.len() > 32 {
        return Err(StoreError::Capacity("plan_tasks"));
    }
    let mut keys = BTreeSet::new();
    for task in &content.tasks {
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
    }
    let bytes = serde_json::to_vec(content).map_err(|_| StoreError::CorruptState)?;
    if bytes.len() > MAX_PLAN_BYTES {
        return Err(StoreError::Capacity("plan_bytes"));
    }
    Ok(bytes)
}

pub(crate) fn content_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
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
