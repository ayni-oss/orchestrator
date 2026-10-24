use super::{StoreError, support};
use crate::model::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProposalInput {
    pub feedback_id: String,
    pub explanation: String,
    pub action: String,
    pub additional_seconds: u32,
    pub additional_tokens: u32,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecoveryInput {
    Proposal(ProposalInput),
    Question { text: String },
    Answer { question_id: String, text: String },
}
impl RecoveryInput {
    pub(super) fn validate(&self) -> Result<(), StoreError> {
        match self {
            Self::Proposal(p) => {
                support::validate_id(&p.feedback_id, "feedback_id")?;
                support::validate_text(&p.explanation, 4096, false, "explanation")?;
                if !["continue", "repair_quality", "replace_attempt"].contains(&p.action.as_str()) {
                    return Err(StoreError::InvalidInput("action"));
                }
                if !(1..=86400).contains(&p.additional_seconds)
                    || !(1..=1_000_000).contains(&p.additional_tokens)
                {
                    return Err(StoreError::InvalidInput("additional_allowance"));
                }
                Ok(())
            }
            Self::Question { text } => support::validate_text(text, 4096, false, "question"),
            Self::Answer { question_id, text } => {
                support::validate_id(question_id, "question_id")?;
                support::validate_text(text, 4096, false, "answer")
            }
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SavedEvidence {
    pub state: String,
    pub generation: u64,
    pub provider_uncertain: bool,
    pub source_commit: String,
    pub runtime_image: String,
    pub configuration_revision_id: String,
    pub patch_available: bool,
    pub patch_digest: String,
    pub outcome_digest: String,
    pub quality_digest: String,
    pub quality_state: String,
    pub limitations: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryProposal {
    pub request: ProposalInput,
    pub safe_reason: String,
    pub evidence: SavedEvidence,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecoveryContent {
    Proposal(Box<RecoveryProposal>),
    Question { text: String },
    Answer { question_id: String, text: String },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryRecord {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub kind: String,
    pub content: RecoveryContent,
    pub content_digest: String,
    pub created_at: Timestamp,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryPage {
    pub records: Vec<RecoveryRecord>,
    pub next_after_id: String,
    pub confirmations: Vec<super::RecoveryConfirmation>,
    pub question_answers: Vec<RecoveryRecord>,
}
