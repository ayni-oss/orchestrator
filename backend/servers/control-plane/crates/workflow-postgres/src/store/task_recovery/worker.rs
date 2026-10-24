//! Accepted worker questions and recommendations retain provenance, never execution consent.
use super::{
    ProposalInput, RecoveryContent, RecoveryProposal, StoreError, decode, encode, frozen_evidence,
    support,
};
use crate::plans::PlanScope;
use ayni_orchestrator_events::task_execution::{TaskAnswer, TaskOutcome, digest};
use tokio_postgres::Transaction;

pub(in crate::store) async fn retain_worker_review(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    task: &str,
    attempt: &str,
) -> Result<(), StoreError> {
    let row = tx
        .query_one(
            "SELECT state,outcome FROM workflow_task_runs WHERE attempt_id=$1",
            &[&attempt],
        )
        .await?;
    let state: String = row.get(0);
    if state != "failed" {
        return Ok(());
    }
    let Some(bytes) = row.get::<_, Option<Vec<u8>>>(1) else {
        return Ok(());
    };
    let outcome: TaskOutcome = decode(&bytes)?;
    let Some(advice) = outcome.recovery_advice else {
        return Ok(());
    };
    if !advice.validate() {
        return Err(StoreError::CorruptState);
    }
    let feedback = tx.query_one("SELECT id,safe_reason FROM workflow_task_feedback WHERE attempt_id=$1 AND category='execution'", &[&attempt]).await?;
    let evidence = frozen_evidence(tx, scope, task, attempt).await?;
    let proposal = RecoveryContent::Proposal(Box::new(RecoveryProposal {
        request: ProposalInput {
            feedback_id: feedback.get(0),
            explanation: advice.explanation,
            action: advice.action,
            additional_seconds: advice.additional_seconds,
            additional_tokens: advice.additional_tokens,
        },
        safe_reason: feedback.get(1),
        evidence,
    }));
    for (kind, content) in [
        (
            "question",
            RecoveryContent::Question {
                text: advice.question,
            },
        ),
        ("proposal", proposal),
    ] {
        let bytes = encode(&content)?;
        tx.execute("INSERT INTO workflow_task_recovery(id,workspace_id,project_id,conversation_id,task_id,attempt_id,idempotency_key,request_digest,kind,content) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT(attempt_id,idempotency_key) DO NOTHING",
            &[&support::new_id(), &scope.workspace, &scope.project, &scope.conversation, &task, &attempt,
            &format!("worker:{kind}"), &digest(&bytes).as_bytes(), &kind, &bytes]).await?;
    }
    Ok(())
}

/// Freeze canonical answers into the new approved segment; unanswered worker questions block it.
pub(super) async fn answers(
    tx: &Transaction<'_>,
    attempt: &str,
) -> Result<Vec<TaskAnswer>, StoreError> {
    let rows = tx.query("SELECT q.id,q.content,a.content,q.idempotency_key FROM workflow_task_recovery q LEFT JOIN workflow_task_recovery a ON a.question_id=q.id WHERE q.attempt_id=$1 AND q.kind='question' ORDER BY q.id LIMIT 33", &[&attempt]).await?;
    if rows.len() > 32 {
        return Err(StoreError::InvalidInput("question_context_limit"));
    }
    let mut answers = Vec::new();
    for row in rows {
        let question: RecoveryContent = decode(&row.get::<_, Vec<u8>>(1))?;
        let Some(bytes) = row.get::<_, Option<Vec<u8>>>(2) else {
            if row.get::<_, String>(3).starts_with("worker:") {
                return Err(StoreError::TaskExecutionConflict);
            }
            continue;
        };
        let answer: RecoveryContent = decode(&bytes)?;
        let (
            RecoveryContent::Question { text: question },
            RecoveryContent::Answer { text: answer, .. },
        ) = (question, answer)
        else {
            return Err(StoreError::CorruptState);
        };
        answers.push(TaskAnswer {
            question_id: row.get(0),
            question,
            answer,
        });
    }
    Ok(answers)
}
