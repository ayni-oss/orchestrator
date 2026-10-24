//! Immutable review records; exact confirmation alone admits a linked execution segment.
mod confirmation;
mod records;
mod worker;
use super::{StoreError, WorkflowStore, support};
use crate::plans::PlanScope;
use ayni_orchestrator_events::task_execution::{TaskCommand, TaskOutcome, digest};
use ayni_orchestrator_events::task_quality::QualityOutcome;
pub use confirmation::{ConfirmRecovery, RecoveryConfirmation};
pub use records::*;
use tokio_postgres::{Row, Transaction};
pub(super) use worker::retain_worker_review;

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(bytes).map_err(|_| StoreError::CorruptState)
}
fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(value).map_err(|_| StoreError::CorruptState)
}
fn projection(row: &Row) -> Result<RecoveryRecord, StoreError> {
    Ok(RecoveryRecord {
        id: row.get(0),
        workspace_id: row.get(1),
        project_id: row.get(2),
        conversation_id: row.get(3),
        task_id: row.get(4),
        attempt_id: row.get(5),
        kind: row.get(6),
        content: decode(&row.get::<_, Vec<u8>>(7))?,
        content_digest: digest(&row.get::<_, Vec<u8>>(7)),
        created_at: support::timestamp(row.get(8), row.get(9))?,
        author: if row.get::<_, String>(10).starts_with("worker:") {
            "agent"
        } else {
            "local-operator"
        }
        .into(),
    })
}
const SELECT: &str = "SELECT id,workspace_id,project_id,conversation_id,task_id,attempt_id,kind,content,workflow_ts_seconds(created_at),workflow_ts_nanos(created_at),idempotency_key FROM workflow_task_recovery";

async fn frozen_evidence(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    task: &str,
    attempt: &str,
) -> Result<SavedEvidence, StoreError> {
    // Serialize review writes and terminal worker reconciliation on the attempt row.
    let r=tx.query_opt("SELECT command,state,generation,provider_state,outcome,(SELECT outcome FROM workflow_task_quality WHERE attempt_id=$5) FROM workflow_task_runs WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND task_id=$4 AND attempt_id=$5 FOR UPDATE", &[&scope.workspace,&scope.project,&scope.conversation,&task,&attempt]).await?.ok_or(StoreError::NotFound)?;
    let state: String = r.get(1);
    if ![
        "completed",
        "failed",
        "interrupted",
        "cancelled",
        "uncertain",
    ]
    .contains(&state.as_str())
    {
        return Err(StoreError::TaskExecutionConflict);
    }
    let command: TaskCommand = decode(&r.get::<_, Vec<u8>>(0))?;
    let outcome_bytes: Option<Vec<u8>> = r.get(4);
    let quality_bytes: Option<Vec<u8>> = r.get(5);
    let outcome = outcome_bytes
        .as_ref()
        .map(|b| decode::<TaskOutcome>(b))
        .transpose()?;
    let quality = quality_bytes
        .as_ref()
        .map(|b| decode::<QualityOutcome>(b))
        .transpose()?;
    let provider: String = r.get(3);
    let patch = ayni_orchestrator_events::task_execution::saved_patch(&command, outcome.as_ref());
    let patch_available = !patch.is_empty();
    let patch_digest = if outcome.is_some() || command.recovery.is_some() {
        digest(patch.as_bytes())
    } else {
        String::new()
    };
    Ok(SavedEvidence {
        state, generation: support::unsigned(r.get(2))?,
        provider_uncertain: provider == "uncertain" || outcome.as_ref().is_some_and(|o|o.state=="uncertain"),
        source_commit: command.source_commit, runtime_image: command.runtime_image,
        configuration_revision_id: command.configuration_revision_id,
        patch_available,
        patch_digest,
        outcome_digest: outcome_bytes.as_ref().map(|b|digest(b)).unwrap_or_default(),
        quality_digest: quality_bytes.as_ref().map(|b|digest(b)).unwrap_or_default(),
        quality_state: quality.as_ref().map(|q|q.state.clone()).unwrap_or_default(),
        limitations: "Only accepted patches and quality artifacts are retained evidence. Unreported workspace files and provider progress are unavailable; saved changes have not been verified for continuation.".into(),
    })
}

async fn require_attempt(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    task: &str,
    attempt: &str,
) -> Result<(), StoreError> {
    tx.query_opt("SELECT attempt_id FROM workflow_task_runs WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND task_id=$4 AND attempt_id=$5 FOR UPDATE", &[&scope.workspace,&scope.project,&scope.conversation,&task,&attempt]).await?.ok_or(StoreError::NotFound)?;
    Ok(())
}
impl WorkflowStore {
    /// Retain an exact operator proposal, question or first answer. Replays return the original record.
    /// # Errors
    /// Rejects foreign scope, changed command keys, invalid allowances and unavailable recovery evidence.
    pub async fn record_task_recovery(
        &self,
        scope: PlanScope<'_>,
        task: &str,
        attempt: &str,
        key: &str,
        input: &RecoveryInput,
    ) -> Result<RecoveryRecord, StoreError> {
        validate(scope, task, attempt)?;
        support::validate_key(key)?;
        if key.starts_with("worker:") {
            return Err(StoreError::InvalidInput("reserved_command_key"));
        }
        input.validate()?;
        let request_digest = digest(&encode(input)?);
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        require_attempt(&tx, scope, task, attempt).await?;
        if let Some(row) = tx
            .query_opt(
                &format!("{SELECT} WHERE attempt_id=$1 AND idempotency_key=$2"),
                &[&attempt, &key],
            )
            .await?
        {
            let retained: Vec<u8> = tx
                .query_one(
                    "SELECT request_digest FROM workflow_task_recovery WHERE id=$1",
                    &[&row.get::<_, String>(0)],
                )
                .await?
                .get(0);
            if retained != request_digest.as_bytes() {
                return Err(StoreError::IdempotencyConflict);
            }
            let result = projection(&row)?;
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let (kind, content, question_id) = match input {
            RecoveryInput::Proposal(p) => {
                let feedback=tx.query_opt("SELECT safe_reason FROM workflow_task_feedback WHERE id=$1 AND workspace_id=$2 AND project_id=$3 AND conversation_id=$4 AND task_id=$5 AND attempt_id=$6", &[&p.feedback_id,&scope.workspace,&scope.project,&scope.conversation,&task,&attempt]).await?.ok_or(StoreError::NotFound)?;
                let evidence = frozen_evidence(&tx, scope, task, attempt).await?;
                (
                    "proposal",
                    RecoveryContent::Proposal(Box::new(RecoveryProposal {
                        request: p.clone(),
                        safe_reason: feedback.get(0),
                        evidence,
                    })),
                    None,
                )
            }
            RecoveryInput::Question { text } => (
                "question",
                RecoveryContent::Question { text: text.clone() },
                None,
            ),
            RecoveryInput::Answer { question_id, text } => {
                tx.query_opt("SELECT id FROM workflow_task_recovery WHERE id=$1 AND attempt_id=$2 AND kind='question'", &[&question_id,&attempt]).await?.ok_or(StoreError::NotFound)?;
                if tx
                    .query_opt(
                        "SELECT id FROM workflow_task_recovery WHERE question_id=$1",
                        &[&question_id],
                    )
                    .await?
                    .is_some()
                {
                    return Err(StoreError::TaskExecutionConflict);
                }
                (
                    "answer",
                    RecoveryContent::Answer {
                        question_id: question_id.clone(),
                        text: text.clone(),
                    },
                    Some(question_id.clone()),
                )
            }
        };
        let bytes = encode(&content)?;
        let row=tx.query_one("WITH inserted AS (INSERT INTO workflow_task_recovery(id,workspace_id,project_id,conversation_id,task_id,attempt_id,idempotency_key,request_digest,kind,question_id,content) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING *) SELECT id,workspace_id,project_id,conversation_id,task_id,attempt_id,kind,content,workflow_ts_seconds(created_at),workflow_ts_nanos(created_at),idempotency_key FROM inserted", &[&support::new_id(),&scope.workspace,&scope.project,&scope.conversation,&task,&attempt,&key,&request_digest.as_bytes(),&kind,&question_id,&bytes]).await?;
        let result = projection(&row)?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Bounded origin-scoped immutable review history. This read grants no recovery authority.
    /// # Errors
    /// Rejects missing or foreign attempts and invalid pagination bounds.
    pub async fn list_task_recovery(
        &self,
        scope: PlanScope<'_>,
        task: &str,
        attempt: &str,
        after: &str,
        limit: u32,
    ) -> Result<RecoveryPage, StoreError> {
        validate(scope, task, attempt)?;
        if !after.is_empty() {
            support::validate_id(after, "after_id")?;
        }
        let limit = match limit {
            0 => 20,
            1..=32 => limit,
            _ => return Err(StoreError::InvalidInput("limit")),
        };
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        // Reads use a scoped existence check without locking or modifying the attempt.
        tx.query_opt("SELECT attempt_id FROM workflow_task_runs WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND task_id=$4 AND attempt_id=$5", &[&scope.workspace,&scope.project,&scope.conversation,&task,&attempt]).await?.ok_or(StoreError::NotFound)?;
        let mut records = tx
            .query(
                &format!("{SELECT} WHERE attempt_id=$1 AND id>$2 ORDER BY id LIMIT $3"),
                &[&attempt, &after, &(i64::from(limit) + 1)],
            )
            .await?
            .iter()
            .map(projection)
            .collect::<Result<Vec<_>, _>>()?;
        let next_after_id = if records.len() > limit as usize {
            records.pop();
            records.last().map_or_else(String::new, |r| r.id.clone())
        } else {
            String::new()
        };
        let ids: Vec<_> = records
            .iter()
            .filter(|r| r.kind == "question")
            .map(|r| r.id.as_str())
            .collect();
        let question_answers = tx
            .query(
                &format!("{SELECT} WHERE attempt_id=$1 AND question_id=ANY($2) ORDER BY id"),
                &[&attempt, &ids],
            )
            .await?
            .iter()
            .map(projection)
            .collect::<Result<Vec<_>, _>>()?;
        let confirmations = tx
            .query(
                "SELECT result FROM workflow_task_recovery_confirmations WHERE parent_attempt_id=$1",
                &[&attempt],
            )
            .await?
            .iter()
            .map(|r| decode(&r.get::<_, Vec<u8>>(0)))
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit().await?;
        driver.abort();
        Ok(RecoveryPage {
            records,
            next_after_id,
            confirmations,
            question_answers,
        })
    }
}
fn validate(scope: PlanScope<'_>, task: &str, attempt: &str) -> Result<(), StoreError> {
    support::validate_scope(scope.workspace, scope.project, scope.conversation)?;
    support::validate_id(task, "task_id")?;
    support::validate_id(attempt, "attempt_id")
}
