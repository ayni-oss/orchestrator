//! Operator-authorized execution preparation, separate from admission and dispatch.
use crate::{StoreError, model::Timestamp};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionInputs {
    pub repository: String,
    pub source_ref: String,
    pub source_commit: String,
    pub lock_digest: String,
    pub runtime_image: String,
    pub max_seconds: u32,
    pub max_tokens: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upstream_selections: Vec<UpstreamSelection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamSelection {
    pub task_id: String,
    pub attempt_id: String,
    pub commit_sha: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskExecutionPreparation {
    pub command_id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub inputs: ExecutionInputs,
    pub blockers: Vec<String>,
    pub configuration_revision_id: String,
    pub conversation_revision: u64,
    pub resume_cursor: String,
    pub created_at: Timestamp,
}

/// Validate bounded operator evidence without interpreting Ayni lock contents.
///
/// # Errors
/// Rejects malformed source identities, runtime pins or excessive budgets.
pub fn validate_inputs(inputs: &ExecutionInputs) -> Result<(), StoreError> {
    for value in [
        &inputs.repository,
        &inputs.source_ref,
        &inputs.runtime_image,
    ] {
        if value.len() > 512 || value.chars().any(char::is_control) {
            return Err(StoreError::InvalidInput("execution_inputs"));
        }
    }
    if (!inputs.source_commit.is_empty() && !hex(&inputs.source_commit, 40))
        || (!inputs.lock_digest.is_empty() && !digest(&inputs.lock_digest))
        || (!inputs.runtime_image.is_empty()
            && !inputs
                .runtime_image
                .rsplit_once('@')
                .is_some_and(|(name, pin)| {
                    !name.is_empty()
                        && name.bytes().all(|b| {
                            b.is_ascii_lowercase() || b.is_ascii_digit() || b"._/:-".contains(&b)
                        })
                        && digest(pin)
                }))
        || inputs.max_seconds > 3600
        || inputs.max_tokens > 200_000
    {
        return Err(StoreError::InvalidInput("execution_inputs"));
    }
    if inputs.upstream_selections.len() > 31 {
        return Err(StoreError::Capacity("upstream_selections"));
    }
    let mut tasks = std::collections::BTreeSet::new();
    for selection in &inputs.upstream_selections {
        for value in [&selection.task_id, &selection.attempt_id] {
            if !uuid::Uuid::parse_str(value)
                .is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == *value)
            {
                return Err(StoreError::InvalidInput("upstream_identity"));
            }
        }
        if !hex(&selection.commit_sha, 40) || !tasks.insert(&selection.task_id) {
            return Err(StoreError::InvalidInput("upstream_selection"));
        }
    }
    Ok(())
}
fn hex(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|value| hex(value, 64))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_mutable_runtime_names_and_unbounded_execution() {
        let mut inputs = ExecutionInputs::default();
        assert!(validate_inputs(&inputs).is_ok());
        inputs.runtime_image = "runtime:latest".into();
        assert!(validate_inputs(&inputs).is_err());
        inputs.runtime_image = format!("runtime@sha256:{}", "a".repeat(64));
        inputs.max_seconds = 3601;
        assert!(validate_inputs(&inputs).is_err());
        inputs.max_seconds = 3600;
        inputs.max_tokens = 200_001;
        assert!(validate_inputs(&inputs).is_err());
        inputs.max_tokens = 200_000;
        assert!(validate_inputs(&inputs).is_ok());
    }
}
