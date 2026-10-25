//! Provisional text is scoped, fenced and never a source of transcript authority.
use super::{StoreError, WorkflowStore, support};
use crate::model::Timestamp;
use ayni_orchestrator_events::{progress::ExecutionProgress, v1::ConversationTurnBinding};
use tokio_postgres::Transaction;

pub struct ObservedProgress {
    pub turn_id: String,
    pub session_id: String,
    pub generation: u64,
    pub progress: ExecutionProgress,
    pub stage_observed_at: Timestamp,
    pub observed_at: Timestamp,
}

pub(super) async fn retain(
    tx: &Transaction<'_>,
    binding: &ConversationTurnBinding,
    session_id: &str,
    progress: &ExecutionProgress,
) -> Result<(), StoreError> {
    if !progress.valid() {
        return Err(StoreError::InvalidInput("execution_progress"));
    }
    let generation = i64::try_from(binding.executor_generation)
        .map_err(|_| StoreError::InvalidInput("generation"))?;
    let sequence =
        i64::try_from(progress.sequence).map_err(|_| StoreError::InvalidInput("sequence"))?;
    let round = i32::try_from(progress.round).map_err(|_| StoreError::InvalidInput("round"))?;
    let prior = tx.query_opt("SELECT session_id,generation,sequence,round,call_id,stage,tool_name,provisional_text,tool_invocation_id FROM workflow_execution_progress WHERE turn_id=$1", &[&binding.turn_id]).await?;
    if let Some(row) = &prior {
        let current: i64 = row.get(2);
        if row.get::<_, String>(0) != session_id || row.get::<_, i64>(1) != generation {
            return Err(StoreError::InvalidInput("stale_progress"));
        }
        if sequence < current {
            return Ok(());
        }
        if sequence == current {
            if row.get::<_, i32>(3) != round
                || row.get::<_, String>(4) != progress.call_id
                || row.get::<_, String>(5) != progress.stage
                || row.get::<_, String>(6) != progress.tool_name
                || row.get::<_, String>(7) != progress.provisional_text
                || row.get::<_, String>(8) != progress.tool_invocation_id
            {
                return Err(StoreError::InvalidInput("changed_progress"));
            }
            return Ok(());
        }
        if round < row.get::<_, i32>(3) {
            return Err(StoreError::InvalidInput("stale_round"));
        }
    }
    let changed_stage = prior.as_ref().is_none_or(|row| {
        row.get::<_, i32>(3) != round
            || row.get::<_, String>(4) != progress.call_id
            || row.get::<_, String>(5) != progress.stage
            || row.get::<_, String>(6) != progress.tool_name
            || row.get::<_, String>(8) != progress.tool_invocation_id
    });
    tx.execute("INSERT INTO workflow_execution_progress(turn_id,session_id,generation,sequence,round,call_id,stage,tool_name,provisional_text,tool_invocation_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT(turn_id) DO UPDATE SET sequence=excluded.sequence,round=excluded.round,call_id=excluded.call_id,stage=excluded.stage,tool_name=excluded.tool_name,tool_invocation_id=excluded.tool_invocation_id,provisional_text=excluded.provisional_text,observed_at=clock_timestamp(),stage_observed_at=CASE WHEN $11 THEN clock_timestamp() ELSE workflow_execution_progress.stage_observed_at END", &[&binding.turn_id,&session_id,&generation,&sequence,&round,&progress.call_id,&progress.stage,&progress.tool_name,&progress.provisional_text,&progress.tool_invocation_id,&changed_stage]).await?;
    if changed_stage {
        let stage = match progress.stage.as_str() {
            "preparation" => "preparation",
            "tool" => "tool_execution",
            "scripted" => "execution",
            _ => "provider_request",
        };
        super::acceptance::append_activity_detail(
            tx,
            binding,
            stage,
            "processing",
            "",
            "execution_progress_observed",
            &format!(
                "Round {} · call {} · tool {} · invocation {}",
                progress.round + 1,
                progress.call_id,
                progress.tool_name,
                progress.tool_invocation_id
            ),
        )
        .await?;
    }
    Ok(())
}

impl WorkflowStore {
    /// Return current provisional snapshots only for active, uncancelled generations.
    /// # Errors
    /// Returns invalid scope or unavailable retained-state errors.
    pub async fn list_execution_progress(
        &self,
        workspace: &str,
        project: &str,
        conversation: &str,
    ) -> Result<Vec<ObservedProgress>, StoreError> {
        support::validate_scope(workspace, project, conversation)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        support::conversation(&tx, workspace, project, conversation).await?;
        let rows = tx.query("SELECT p.turn_id,p.session_id,p.generation,p.sequence,p.round,p.call_id,p.stage,p.tool_name,p.provisional_text,workflow_ts_seconds(p.stage_observed_at),workflow_ts_nanos(p.stage_observed_at),workflow_ts_seconds(p.observed_at),workflow_ts_nanos(p.observed_at),p.tool_invocation_id FROM workflow_execution_progress p JOIN workflow_turns t ON t.id=p.turn_id JOIN workflow_sessions s ON s.id=p.session_id AND s.generation=p.generation WHERE t.workspace_id=$1 AND t.project_id=$2 AND t.conversation_id=$3 AND t.execution_state='processing' AND t.failure_reason <> 'provider_uncertain' AND s.state NOT IN ('expired','replaced') AND NOT EXISTS(SELECT 1 FROM workflow_cancellation_receipts c WHERE c.turn_id=t.id AND c.cancellation_requested) ORDER BY t.sequence LIMIT 100", &[&workspace,&project,&conversation]).await?;
        let result = rows
            .into_iter()
            .map(|row| {
                Ok(ObservedProgress {
                    turn_id: row.get(0),
                    session_id: row.get(1),
                    generation: support::unsigned(row.get(2))?,
                    progress: ExecutionProgress {
                        sequence: support::unsigned(row.get(3))?,
                        round: u32::try_from(row.get::<_, i32>(4))
                            .map_err(|_| StoreError::CorruptState)?,
                        call_id: row.get(5),
                        stage: row.get(6),
                        tool_name: row.get(7),
                        provisional_text: row.get(8),
                        tool_invocation_id: row.get(13),
                    },
                    stage_observed_at: Timestamp {
                        seconds: row.get(9),
                        nanos: i32::try_from(row.get::<_, i64>(10))
                            .map_err(|_| StoreError::CorruptState)?,
                    },
                    observed_at: Timestamp {
                        seconds: row.get(11),
                        nanos: i32::try_from(row.get::<_, i64>(12))
                            .map_err(|_| StoreError::CorruptState)?,
                    },
                })
            })
            .collect();
        tx.commit().await?;
        driver.abort();
        result
    }
}
