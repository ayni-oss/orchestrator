//! Scoped preparation receipts and frozen attempts. No broker or worker side effects.
use super::{StoreError, WorkflowStore, observation, support};
use crate::{
    execution::{ExecutionInputs, TaskExecutionPreparation, validate_inputs},
    plans::PlanScope,
};
use tokio_postgres::GenericClient;

impl WorkflowStore {
    /// Retain a blocked preparation or one immutable prepared attempt under operator authority.
    ///
    /// # Errors
    /// Rejects scope, conflicting retries, changed approved inputs or a second attempt.
    pub async fn prepare_task_execution(
        &self,
        scope: PlanScope<'_>,
        key: &str,
        task: &str,
        inputs: &ExecutionInputs,
    ) -> Result<TaskExecutionPreparation, StoreError> {
        support::validate_scope(scope.workspace, scope.project, scope.conversation)?;
        support::validate_key(key)?;
        support::validate_id(task, "task_id")?;
        validate_inputs(inputs)?;
        let bytes = serde_json::to_vec(inputs).map_err(|_| StoreError::CorruptState)?;
        let digest = support::request_digest(
            "PrepareTaskExecution",
            &[
                scope.workspace.as_bytes(),
                scope.project.as_bytes(),
                scope.conversation.as_bytes(),
                task.as_bytes(),
                &bytes,
            ],
        );
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let owner = tx.query_opt("SELECT revision FROM workflow_conversations WHERE workspace_id=$1 AND project_id=$2 AND id=$3 FOR UPDATE", &[&scope.workspace,&scope.project,&scope.conversation]).await?.ok_or(StoreError::NotFound)?;
        if let Some(row) = tx.query_opt("SELECT request_digest,result FROM workflow_task_preparations WHERE conversation_id=$1 AND idempotency_key=$2", &[&scope.conversation,&key]).await? {
            support::ensure_digest(&row.get::<_,Vec<u8>>(0), &digest)?;
            let result = decode(&row.get::<_,Vec<u8>>(1))?;
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let row = tx.query_opt("SELECT specification,plan_id,version_id,content_digest,approval_id FROM workflow_tasks WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND id=$4", &[&scope.workspace,&scope.project,&scope.conversation,&task]).await?.ok_or(StoreError::NotFound)?;
        if tx
            .query_opt(
                "SELECT 1 FROM workflow_task_attempts WHERE task_id=$1",
                &[&task],
            )
            .await?
            .is_some()
        {
            return Err(StoreError::TaskAlreadyPrepared);
        }
        let count: i64 = tx
            .query_one(
                "SELECT count(*) FROM workflow_task_preparations WHERE task_id=$1",
                &[&task],
            )
            .await?
            .get(0);
        if count >= 32 {
            return Err(StoreError::Capacity("task_preparations"));
        }
        let revision = owner
            .get::<_, i64>(0)
            .checked_add(1)
            .ok_or(StoreError::CorruptState)?;
        let result = preparation_result(&tx, scope, task, inputs, &row, revision).await?;
        let command = &result.command_id;
        let attempt = &result.attempt_id;
        let configuration = &result.configuration_revision_id;
        let result_bytes = serde_json::to_vec(&result).map_err(|_| StoreError::CorruptState)?;
        tx.execute("INSERT INTO workflow_task_preparations (command_id,workspace_id,project_id,conversation_id,task_id,idempotency_key,request_digest,result) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)", &[&command,&scope.workspace,&scope.project,&scope.conversation,&task,&key,&&digest[..],&result_bytes]).await?;
        if !attempt.is_empty() {
            tx.execute("INSERT INTO workflow_task_attempts (id,workspace_id,project_id,conversation_id,plan_id,version_id,content_digest,approval_id,task_id,preparation_command_id,configuration_revision_id,inputs,state) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'prepared')", &[&attempt,&scope.workspace,&scope.project,&scope.conversation,&row.get::<_,String>(1),&row.get::<_,String>(2),&row.get::<_,String>(3),&row.get::<_,String>(4),&task,&command,&configuration,&bytes]).await?;
        }
        observation::notify(&tx, scope.conversation).await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }

    /// Read the latest retained preparation in the task's immutable originating scope.
    ///
    /// # Errors
    /// Rejects missing scope and corrupt retained evidence.
    pub async fn get_task_execution(
        &self,
        scope: PlanScope<'_>,
        task: &str,
    ) -> Result<Option<TaskExecutionPreparation>, StoreError> {
        self.get_task(scope, task).await?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let mut result: Option<TaskExecutionPreparation> = client.query_opt("SELECT result FROM workflow_task_preparations WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND task_id=$4 ORDER BY command_id DESC LIMIT 1", &[&scope.workspace,&scope.project,&scope.conversation,&task]).await?.map(|row| decode(&row.get::<_,Vec<u8>>(0))).transpose()?;
        if let Some(ref mut preparation) = result
            && let Some(row) = client.query_opt("SELECT c.result,a.inputs,a.configuration_revision_id,workflow_ts_seconds(r.created_at),workflow_ts_nanos(r.created_at) FROM workflow_task_recovery_confirmations c JOIN workflow_task_runs r ON r.attempt_id=c.attempt_id JOIN workflow_task_attempts a ON a.id=r.attempt_id WHERE r.workspace_id=$1 AND r.project_id=$2 AND r.conversation_id=$3 AND r.task_id=$4 ORDER BY r.created_at DESC,r.attempt_id DESC LIMIT 1", &[&scope.workspace,&scope.project,&scope.conversation,&task]).await? {
                let confirmation: super::task_recovery::RecoveryConfirmation = serde_json::from_slice(&row.get::<_,Vec<u8>>(0)).map_err(|_|StoreError::CorruptState)?;
                preparation.command_id=confirmation.command_id;preparation.attempt_id=confirmation.attempt_id;
                preparation.inputs=serde_json::from_slice(&row.get::<_,Vec<u8>>(1)).map_err(|_|StoreError::CorruptState)?;
                preparation.configuration_revision_id=row.get(2);preparation.conversation_revision=confirmation.conversation_revision;preparation.resume_cursor=confirmation.resume_cursor;
                preparation.created_at=support::timestamp(row.get(3),row.get(4))?;
        }
        driver.abort();
        Ok(result)
    }
}
fn decode(bytes: &[u8]) -> Result<TaskExecutionPreparation, StoreError> {
    serde_json::from_slice(bytes).map_err(|_| StoreError::CorruptState)
}
fn blockers(
    task: &crate::plans::PlanTaskSpecification,
    inputs: &ExecutionInputs,
) -> Result<Vec<String>, StoreError> {
    if (!inputs.repository.is_empty() && inputs.repository != task.repository)
        || (!inputs.source_ref.is_empty() && inputs.source_ref != task.source_ref)
    {
        return Err(StoreError::InvalidInput("approved_execution_inputs"));
    }
    let mut blockers = task.unresolved_inputs.clone();
    if inputs.repository.is_empty()
        || inputs.source_ref.is_empty()
        || inputs.source_commit.is_empty()
    {
        blockers.push("Repository and source verification is required".into());
    }
    if inputs.lock_digest.is_empty() {
        blockers.push("A frozen Ayni lock is required".into());
    }
    if inputs.runtime_image.is_empty() {
        blockers.push("A digest-pinned Ayni runtime must be selected".into());
    }
    if inputs.max_seconds == 0 || inputs.max_tokens == 0 {
        blockers.push("Explicit execution time and token budgets are required".into());
    }
    Ok(blockers)
}
async fn record_update(
    tx: &(impl GenericClient + Sync),
    scope: PlanScope<'_>,
    revision: i64,
    task: &str,
    attempt: &str,
) -> Result<String, StoreError> {
    tx.execute(
        "UPDATE workflow_conversations SET revision=$2 WHERE id=$1",
        &[&scope.conversation, &revision],
    )
    .await?;
    let activity = support::new_id();
    let reason = if attempt.is_empty() {
        "task_execution_blocked"
    } else {
        "task_execution_prepared"
    };
    tx.execute("INSERT INTO workflow_activities (id,workspace_id,project_id,conversation_id,conversation_revision,stage,presentation_state,safe_reason) VALUES ($1,$2,$3,$4,$5,'admission','accepted',$6)", &[&activity,&scope.workspace,&scope.project,&scope.conversation,&revision,&reason]).await?;
    let affected = if attempt.is_empty() {
        vec![task]
    } else {
        vec![task, attempt]
    };
    let row = tx.query_one("INSERT INTO workflow_updates (id,activity_id,workspace_id,project_id,conversation_id,conversation_revision,kind,affected_resource_ids,presentation_state,safe_reason) VALUES ($1,$2,$3,$4,$5,$6,'task_execution_prepared',$7,'accepted',$8) RETURNING cursor", &[&support::new_id(),&activity,&scope.workspace,&scope.project,&scope.conversation,&revision,&affected,&reason]).await?;
    Ok(support::cursor(scope.conversation, row.get(0)))
}

async fn preparation_result(
    tx: &tokio_postgres::Transaction<'_>,
    scope: PlanScope<'_>,
    task: &str,
    inputs: &ExecutionInputs,
    row: &tokio_postgres::Row,
    revision: i64,
) -> Result<TaskExecutionPreparation, StoreError> {
    let specification: crate::plans::PlanTaskSpecification =
        serde_json::from_slice(&row.get::<_, Vec<u8>>(0)).map_err(|_| StoreError::CorruptState)?;
    let configuration = tx
        .query_opt(
            "SELECT revision_id FROM workflow_agent_configuration_heads WHERE workspace_id=$1",
            &[&scope.workspace],
        )
        .await?
        .map(|row| row.get::<_, String>(0))
        .unwrap_or_default();
    let mut blockers = blockers(&specification, inputs)?;
    if configuration.is_empty() {
        blockers.push("Workspace agent configuration must be selected".into());
    }
    let command = support::new_id();
    let attempt = if blockers.is_empty() {
        support::new_id()
    } else {
        String::new()
    };
    let update = record_update(tx, scope, revision, task, &attempt).await?;
    let time = tx.query_one("SELECT workflow_ts_seconds(t),workflow_ts_nanos(t) FROM (SELECT clock_timestamp() AS t) AS time", &[]).await?;
    Ok(TaskExecutionPreparation {
        command_id: command,
        task_id: task.to_owned(),
        attempt_id: attempt,
        inputs: inputs.clone(),
        blockers,
        configuration_revision_id: configuration,
        conversation_revision: support::unsigned(revision)?,
        resume_cursor: update,
        created_at: support::timestamp(time.get(0), time.get(1))?,
    })
}
