//! Durable bounded concurrent dispatch queue consumed by the authenticated task supervisor.
//! Queue, ownership and report receipts are `PostgreSQL` authority, not Pod existence.
mod authority;
mod operator;
use super::{StoreError, WorkflowStore, observation, support};
use crate::plans::PlanScope;
use ayni_orchestrator_events::task_execution::{TaskCommand, TaskRun};
use tokio_postgres::{GenericClient, Row, Transaction};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunReceipt {
    pub command_id: String,
    pub attempt_id: String,
    pub conversation_revision: u64,
    pub resume_cursor: String,
}
const SELECT_RUN: &str = "SELECT command,state,generation,pod_uid,COALESCE(workflow_ts_seconds(deadline_at),0),provider_state,outcome,(SELECT outcome FROM workflow_task_quality q WHERE q.attempt_id=workflow_task_runs.attempt_id) FROM workflow_task_runs";
pub(super) async fn lock(tx: &Transaction<'_>) -> Result<(), StoreError> {
    tx.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&(crate::migration_lock_id() + 4)],
    )
    .await?;
    Ok(())
}
fn run_row(row: &Row) -> Result<TaskRun, StoreError> {
    Ok(TaskRun {
        command: decode(&row.get::<_, Vec<u8>>(0))?,
        state: row.get(1),
        generation: support::unsigned(row.get(2))?,
        pod_uid: row.get(3),
        deadline_seconds: row.get(4),
        provider_state: row.get(5),
        quality: row
            .get::<_, Option<Vec<u8>>>(7)
            .map(|b| decode(&b))
            .transpose()?,
        outcome: row
            .get::<_, Option<Vec<u8>>>(6)
            .map(|b| decode(&b))
            .transpose()?,
    })
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(bytes).map_err(|_| StoreError::CorruptState)
}
fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(value).map_err(|_| StoreError::CorruptState)
}
fn origin(command: &TaskCommand) -> PlanScope<'_> {
    PlanScope {
        workspace: &command.workspace_id,
        project: &command.project_id,
        conversation: &command.conversation_id,
    }
}
pub(super) async fn read(
    client: &(impl GenericClient + Sync),
    scope: PlanScope<'_>,
    task: &str,
    attempt: &str,
) -> Result<TaskRun, StoreError> {
    let row=client.query_opt(&format!("{SELECT_RUN} WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND task_id=$4 AND attempt_id=$5"), &[&scope.workspace,&scope.project,&scope.conversation,&task,&attempt]).await?.ok_or(StoreError::NotFound)?;
    run_row(&row)
}
pub(super) async fn update(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    task: &str,
    attempt: &str,
    reason: &str,
) -> Result<(u64, String), StoreError> {
    let row=tx.query_opt("SELECT revision FROM workflow_conversations WHERE workspace_id=$1 AND project_id=$2 AND id=$3 FOR UPDATE", &[&scope.workspace,&scope.project,&scope.conversation]).await?.ok_or(StoreError::NotFound)?;
    let revision = row
        .get::<_, i64>(0)
        .checked_add(1)
        .ok_or(StoreError::CorruptState)?;
    tx.execute(
        "UPDATE workflow_conversations SET revision=$2 WHERE id=$1",
        &[&scope.conversation, &revision],
    )
    .await?;
    let activity = support::new_id();
    tx.execute("INSERT INTO workflow_activities (id,workspace_id,project_id,conversation_id,conversation_revision,stage,presentation_state,safe_reason) VALUES ($1,$2,$3,$4,$5,'execution','accepted',$6)", &[&activity,&scope.workspace,&scope.project,&scope.conversation,&revision,&reason]).await?;
    let ids = vec![task, attempt];
    let row=tx.query_one("INSERT INTO workflow_updates (id,activity_id,workspace_id,project_id,conversation_id,conversation_revision,kind,affected_resource_ids,presentation_state,safe_reason) VALUES ($1,$2,$3,$4,$5,$6,'task_execution_changed',$7,'accepted',$8) RETURNING cursor", &[&support::new_id(),&activity,&scope.workspace,&scope.project,&scope.conversation,&revision,&ids,&reason]).await?;
    if matches!(
        reason,
        "task_result_retained"
            | "task_quality_retained"
            | "task_execution_stopped"
            | "task_deadline_exceeded"
            | "task_cancellation_requested"
    ) {
        tx.execute("UPDATE workflow_task_runs SET assessment_outcome_at=clock_timestamp() WHERE attempt_id=$1", &[&attempt]).await?;
    }
    super::task_feedback::retain(tx, scope, task, attempt, reason).await?;
    super::task_recovery::retain_worker_review(tx, scope, task, attempt).await?;
    observation::notify(tx, scope.conversation).await?;
    Ok((
        support::unsigned(revision)?,
        support::cursor(scope.conversation, row.get(0)),
    ))
}
impl WorkflowStore {
    /// Read one attempt under its immutable task origin.
    /// # Errors
    /// Rejects missing origin or corrupt retained state.
    pub async fn get_task_run(
        &self,
        scope: PlanScope<'_>,
        task: &str,
        attempt: &str,
    ) -> Result<TaskRun, StoreError> {
        support::validate_scope(scope.workspace, scope.project, scope.conversation)?;
        support::validate_id(task, "task_id")?;
        support::validate_id(attempt, "attempt_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let result = read(&client, scope, task, attempt).await;
        driver.abort();
        result
    }
}
