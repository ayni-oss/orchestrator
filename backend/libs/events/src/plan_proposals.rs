//! Strict, bounded provider-authored drafts. Parsing conveys no acceptance authority.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_PLAN_BYTES: usize = 32_768;
pub const MAX_PROPOSAL_BYTES: usize = 40_960;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalContent {
    pub content: PlanContent,
    pub assumptions: Vec<String>,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

fn text(value: &str, max: usize, required: bool) -> Result<(), &'static str> {
    if value.len() > max || value.contains('\0') || (required && value.trim().is_empty()) {
        return Err("plan_content");
    }
    Ok(())
}

fn items(values: &[String]) -> Result<(), &'static str> {
    if values.len() > 16 {
        return Err("plan_items");
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
pub fn canonical_content(content: &PlanContent) -> Result<Vec<u8>, &'static str> {
    text(&content.title, 256, true)?;
    if content.title.trim() != content.title {
        return Err("plan_title");
    }
    text(&content.objective, 4096, true)?;
    text(&content.scope, 4096, true)?;
    text(&content.out_of_scope, 4096, false)?;
    items(&content.constraints)?;
    items(&content.unresolved_questions)?;
    validate_tasks(&content.tasks)?;
    let bytes = serde_json::to_vec(content).map_err(|_| "plan_encoding")?;
    if bytes.len() > MAX_PLAN_BYTES {
        return Err("plan_bytes");
    }
    Ok(bytes)
}

fn validate_tasks(tasks: &[PlanTaskSpecification]) -> Result<(), &'static str> {
    if tasks.len() > 32 {
        return Err("plan_tasks");
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
            return Err("plan_task_key");
        }
        text(&task.objective, 4096, true)?;
        text(&task.scope, 4096, true)?;
        text(&task.out_of_scope, 4096, false)?;
        text(&task.repository, 1024, false)?;
        text(&task.source_ref, 256, false)?;
        text(&task.execution_limits, 2048, false)?;
        items(&task.acceptance_criteria)?;
        items(&task.unresolved_inputs)?;
        if !crate::task_execution::valid_source_paths(&task.source_paths) {
            return Err("task_source_paths");
        }
    }
    validate_graph(tasks, &keys)
}

fn validate_graph(
    tasks: &[PlanTaskSpecification],
    keys: &BTreeSet<&String>,
) -> Result<(), &'static str> {
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
            return Err("plan_task_dependency");
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
            return Err("plan_task_dependency");
        }
    }
    Ok(())
}

/// Validate a complete provider draft without truncation or extracting prose.
///
/// # Errors
/// Rejects unknown/missing fields, malformed JSON, duplicate keys and excess bounds.
pub fn validate_proposal(bytes: &[u8]) -> Result<ProposalContent, &'static str> {
    if bytes.len() > MAX_PROPOSAL_BYTES {
        return Err("proposal_bytes");
    }
    let proposal: ProposalContent = serde_json::from_slice(bytes).map_err(|_| "proposal_shape")?;
    canonical_content(&proposal.content)?;
    items(&proposal.assumptions)?;
    items(&proposal.evidence)?;
    Ok(proposal)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "contract fixtures fail immediately")]
mod tests {
    use super::*;
    use serde_json::json;

    fn proposal() -> serde_json::Value {
        json!({"content":{"title":"Bounded draft","objective":"Define work","scope":"This project","out_of_scope":"Deployment","constraints":[],"unresolved_questions":["Which source revision?"],"tasks":[]},"assumptions":["Source is unverified"],"evidence":["Originating conversation supplied the objective"]})
    }

    #[test]
    fn strict_shape_excludes_scope_selection_and_approval() {
        let original = proposal();
        assert!(validate_proposal(&serde_json::to_vec(&original).unwrap()).is_ok());
        for field in ["conversation_id", "plan_id", "approved"] {
            let mut invalid = original.clone();
            invalid[field] = json!("foreign");
            assert!(validate_proposal(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove("assumptions");
        assert!(validate_proposal(&serde_json::to_vec(&missing).unwrap()).is_err());
        let mut wrong = original;
        wrong["content"]["tasks"] = json!([{"key":"task"}]);
        assert!(validate_proposal(&serde_json::to_vec(&wrong).unwrap()).is_err());
    }

    #[test]
    fn utf8_content_wrapper_and_item_limits_reject_without_truncation() {
        let mut draft = proposal();
        draft["content"]["constraints"] = json!(vec!["é".repeat(1024); 16]);
        assert!(validate_proposal(&serde_json::to_vec(&draft).unwrap()).is_err());
        draft = proposal();
        draft["assumptions"] = json!([" "]);
        assert!(validate_proposal(&serde_json::to_vec(&draft).unwrap()).is_err());
        draft["assumptions"] = json!(vec!["Evidence"; 17]);
        assert!(validate_proposal(&serde_json::to_vec(&draft).unwrap()).is_err());
        assert_eq!(
            validate_proposal(&vec![b' '; MAX_PROPOSAL_BYTES + 1]),
            Err("proposal_bytes")
        );
        let duplicate = br#"{"content":{"title":"a","title":"b","objective":"goal","scope":"scope","out_of_scope":"","constraints":[],"unresolved_questions":[],"tasks":[]},"assumptions":[],"evidence":[]}"#;
        assert!(validate_proposal(duplicate).is_err());
    }
}
