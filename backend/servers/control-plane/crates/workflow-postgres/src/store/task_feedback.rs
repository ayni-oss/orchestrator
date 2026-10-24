//! Immutable actionable task facts retained with the accepted task transition.
use super::{StoreError, WorkflowStore, support};
use crate::{model::Timestamp, plans::PlanScope};
use ayni_orchestrator_events::{task_execution::TaskOutcome, task_quality::QualityOutcome};
use tokio_postgres::{Row, Transaction};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskFeedback {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub category: String,
    pub safe_reason: String,
    pub summary: String,
    pub created_at: Timestamp,
    pub recovery_attempt_id: String,
    pub recovery_state: String,
    pub recovery_quality_state: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedbackPage {
    pub feedback: Vec<TaskFeedback>,
    pub next_after_id: String,
}

// These summaries describe accepted facts, never raw worker output or model text.
fn summary(reason: &str) -> Result<&'static str, StoreError> {
    match reason {
        "execution_failed" => Ok(
            "The task could not finish. Review its retained attempt evidence before deciding how to continue.",
        ),
        "execution_interrupted" => Ok(
            "The task worker stopped before finishing. Retained attempt evidence remains available for inspection.",
        ),
        "provider_uncertain" => Ok(
            "An AI request may have been sent and charged. It will not be retried automatically. Review the retained attempt evidence.",
        ),
        "execution_deadline_exceeded" => Ok(
            "The task reached its time limit. Review its retained attempt evidence before deciding how to continue.",
        ),
        "quality_failed" => Ok(
            "Coding finished, but quality checks failed. The accepted changes and quality evidence are preserved.",
        ),
        "quality_blocked" => Ok(
            "Coding finished, but quality validation could not complete. The accepted changes and available evidence are preserved.",
        ),
        "quality_interrupted" => Ok(
            "Coding finished, but quality validation was interrupted. The accepted changes and available evidence are preserved.",
        ),
        _ => Err(StoreError::CorruptState),
    }
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(bytes).map_err(|_| StoreError::CorruptState)
}

/// Called inside the task transition transaction after ownership and report checks.
/// The first actionable fact in each stage remains immutable across later stop reports.
pub(super) async fn retain(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    task: &str,
    attempt: &str,
    transition: &str,
) -> Result<(), StoreError> {
    let row = tx.query_one(
        "SELECT state,provider_state,outcome,(SELECT outcome FROM workflow_task_quality WHERE attempt_id=$1) FROM workflow_task_runs WHERE attempt_id=$1",
        &[&attempt],
    ).await?;
    let state: String = row.get(0);
    let provider: String = row.get(1);
    let outcome = row
        .get::<_, Option<Vec<u8>>>(2)
        .map(|b| decode::<TaskOutcome>(&b))
        .transpose()?;
    let quality = row
        .get::<_, Option<Vec<u8>>>(3)
        .map(|b| decode::<QualityOutcome>(&b))
        .transpose()?;
    let execution =
        if provider == "uncertain" || outcome.as_ref().is_some_and(|v| v.state == "uncertain") {
            Some("provider_uncertain")
        } else if transition == "task_deadline_exceeded" {
            Some("execution_deadline_exceeded")
        } else if state == "interrupted" {
            Some("execution_interrupted")
        } else if outcome.as_ref().is_some_and(|v| v.state == "failed") {
            Some("execution_failed")
        } else {
            None
        };
    let quality = quality.as_ref().and_then(|q| match q.state.as_str() {
        "failed" => Some("quality_failed"),
        "blocked" => Some("quality_blocked"),
        "interrupted" => Some("quality_interrupted"),
        _ => None,
    });
    for (category, reason) in [("execution", execution), ("quality", quality)] {
        if let Some(reason) = reason {
            tx.execute(
                "INSERT INTO workflow_task_feedback(id,workspace_id,project_id,conversation_id,task_id,attempt_id,category,safe_reason) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(attempt_id,category) DO NOTHING",
                &[&support::new_id(),&scope.workspace,&scope.project,&scope.conversation,&task,&attempt,&category,&reason],
            ).await?;
        }
    }
    Ok(())
}

fn projection(row: &Row) -> Result<TaskFeedback, StoreError> {
    let reason: String = row.get(7);
    Ok(TaskFeedback {
        id: row.get(0),
        workspace_id: row.get(1),
        project_id: row.get(2),
        conversation_id: row.get(3),
        task_id: row.get(4),
        attempt_id: row.get(5),
        category: row.get(6),
        summary: summary(&reason)?.to_owned(),
        safe_reason: reason,
        created_at: support::timestamp(row.get(8), row.get(9))?,
        recovery_attempt_id: row.get(10),
        recovery_state: row.get(11),
        recovery_quality_state: row
            .get::<_, Option<Vec<u8>>>(12)
            .map(|bytes| decode::<QualityOutcome>(&bytes).map(|outcome| outcome.state))
            .transpose()?
            .unwrap_or_default(),
    })
}
impl WorkflowStore {
    /// Read project attention or one exact originating conversation, with bounded keyset pagination.
    /// # Errors
    /// Rejects invalid bounds and missing or foreign parent scopes even when the list is empty.
    pub async fn list_task_feedback(
        &self,
        workspace: &str,
        project: &str,
        conversation: &str,
        after_id: &str,
        limit: u32,
    ) -> Result<FeedbackPage, StoreError> {
        support::validate_id(workspace, "workspace_id")?;
        support::validate_id(project, "project_id")?;
        if !conversation.is_empty() {
            support::validate_id(conversation, "conversation_id")?;
        }
        if !after_id.is_empty() {
            support::validate_id(after_id, "after_id")?;
        }
        let limit = match limit {
            0 => 20,
            1..=32 => limit,
            _ => return Err(StoreError::InvalidInput("limit")),
        };
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        support::require_project(&tx, workspace, project).await?;
        if !conversation.is_empty() {
            support::conversation(&tx, workspace, project, conversation).await?;
        }
        let mut feedback = tx.query(
            "SELECT f.id,f.workspace_id,f.project_id,f.conversation_id,f.task_id,f.attempt_id,f.category,f.safe_reason,workflow_ts_seconds(f.created_at),workflow_ts_nanos(f.created_at),COALESCE(c.attempt_id,''),COALESCE(r.state,''),q.outcome FROM workflow_task_feedback f LEFT JOIN workflow_task_recovery_confirmations c ON c.parent_attempt_id=f.attempt_id LEFT JOIN workflow_task_runs r ON r.attempt_id=c.attempt_id LEFT JOIN workflow_task_quality q ON q.attempt_id=c.attempt_id WHERE f.workspace_id=$1 AND f.project_id=$2 AND ($3='' OR f.conversation_id=$3) AND f.id>$4 ORDER BY f.id LIMIT $5",
            &[&workspace,&project,&conversation,&after_id,&(i64::from(limit)+1)],
        ).await?.iter().map(projection).collect::<Result<Vec<_>, _>>()?;
        let next_after_id = if feedback.len() > limit as usize {
            feedback.pop();
            feedback.last().map_or_else(String::new, |v| v.id.clone())
        } else {
            String::new()
        };
        tx.commit().await?;
        driver.abort();
        Ok(FeedbackPage {
            feedback,
            next_after_id,
        })
    }
}
