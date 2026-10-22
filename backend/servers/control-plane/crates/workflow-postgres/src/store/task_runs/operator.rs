use super::{
    PlanScope, RunReceipt, StoreError, TaskCommand, Transaction, WorkflowStore, decode, encode,
    lock, read, support, update,
};
use crate::{execution::ExecutionInputs, plans::PlanTaskSpecification};
use ayni_orchestrator_events::task_execution::{MAX_SOURCE_PACK, digest};
impl WorkflowStore {
    /// Authorize exactly one frozen task dispatch; transport is a retained supervisor queue.
    /// # Errors
    /// Rejects changed retry bytes, wrong origin, missing preparation, capacity or repeated start.
    pub async fn start_task_execution(
        &self,
        scope: PlanScope<'_>,
        task: &str,
        attempt: &str,
        key: &str,
        pack: &[u8],
    ) -> Result<RunReceipt, StoreError> {
        command_input(scope, task, attempt, key)?;
        if pack.is_empty() || pack.len() > MAX_SOURCE_PACK {
            return Err(StoreError::InvalidInput("source_pack"));
        }
        let hash = support::request_digest(
            "StartTaskExecution",
            &[task.as_bytes(), attempt.as_bytes(), pack],
        );
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        lock(&tx).await?;
        support::conversation(&tx, scope.workspace, scope.project, scope.conversation).await?;
        if let Some(receipt) = receipt(&tx, scope, "StartTaskExecution", key, &hash).await? {
            tx.commit().await?;
            driver.abort();
            return Ok(receipt);
        }
        let command = build_command(&tx, scope, task, attempt, pack).await?;
        if tx
            .query_opt(
                "SELECT 1 FROM workflow_task_runs WHERE attempt_id=$1",
                &[&attempt],
            )
            .await?
            .is_some()
        {
            return Err(StoreError::TaskExecutionConflict);
        }
        let pending: i64 = tx
            .query_one(
                "SELECT count(*) FROM workflow_task_runs WHERE state='queued'",
                &[],
            )
            .await?
            .get(0);
        if pending >= 32 {
            return Err(StoreError::Capacity("task_dispatch"));
        }
        tx.execute("INSERT INTO workflow_task_runs (attempt_id,task_id,workspace_id,project_id,conversation_id,command,source_pack,state) VALUES ($1,$2,$3,$4,$5,$6,$7,'queued')",&[&attempt,&task,&scope.workspace,&scope.project,&scope.conversation,&encode(&command)?,&pack]).await?;
        let result = retain(
            &tx,
            scope,
            task,
            attempt,
            "StartTaskExecution",
            key,
            &hash,
            "task_queued",
        )
        .await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Fence reports immediately; release the serial slot only after workload stop confirmation.
    /// # Errors
    /// Rejects wrong origin, conflicting retry bytes and terminal attempts.
    pub async fn cancel_task_execution(
        &self,
        scope: PlanScope<'_>,
        task: &str,
        attempt: &str,
        key: &str,
    ) -> Result<RunReceipt, StoreError> {
        command_input(scope, task, attempt, key)?;
        let hash = support::request_digest(
            "CancelTaskExecution",
            &[task.as_bytes(), attempt.as_bytes()],
        );
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        lock(&tx).await?;
        support::conversation(&tx, scope.workspace, scope.project, scope.conversation).await?;
        if let Some(receipt) = receipt(&tx, scope, "CancelTaskExecution", key, &hash).await? {
            tx.commit().await?;
            driver.abort();
            return Ok(receipt);
        }
        let run = read(&tx, scope, task, attempt).await?;
        let state = match run.state.as_str() {
            "queued" => "cancelled",
            "claimed" | "reporting" => "cancelling",
            _ => return Err(StoreError::TaskExecutionConflict),
        };
        tx.execute("UPDATE workflow_task_runs SET state=$2,generation=generation+1,provider_state=CASE WHEN provider_state='sending' THEN 'uncertain' ELSE provider_state END WHERE attempt_id=$1",&[&attempt,&state]).await?;
        let result = retain(
            &tx,
            scope,
            task,
            attempt,
            "CancelTaskExecution",
            key,
            &hash,
            "task_cancellation_requested",
        )
        .await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
}
fn command_input(
    scope: PlanScope<'_>,
    task: &str,
    attempt: &str,
    key: &str,
) -> Result<(), StoreError> {
    support::validate_scope(scope.workspace, scope.project, scope.conversation)?;
    support::validate_id(task, "task_id")?;
    support::validate_id(attempt, "attempt_id")?;
    support::validate_key(key)
}
async fn receipt(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    method: &str,
    key: &str,
    hash: &[u8; 32],
) -> Result<Option<RunReceipt>, StoreError> {
    tx.query_opt("SELECT request_digest,command_id,attempt_id,conversation_revision,resume_cursor FROM workflow_task_run_receipts WHERE conversation_id=$1 AND method=$2 AND idempotency_key=$3",&[&scope.conversation,&method,&key]).await?.map(|row|{support::ensure_digest(&row.get::<_,Vec<u8>>(0),hash)?;Ok(RunReceipt{command_id:row.get(1),attempt_id:row.get(2),conversation_revision:support::unsigned(row.get(3))?,resume_cursor:row.get(4)})}).transpose()
}
#[allow(
    clippy::too_many_arguments,
    reason = "one scoped receipt binds method, key, digest and exact observation"
)]
async fn retain(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    task: &str,
    attempt: &str,
    method: &str,
    key: &str,
    hash: &[u8],
    reason: &str,
) -> Result<RunReceipt, StoreError> {
    let (revision, cursor) = update(tx, scope, task, attempt, reason).await?;
    let command = support::new_id();
    let signed = i64::try_from(revision).map_err(|_| StoreError::CorruptState)?;
    tx.execute("INSERT INTO workflow_task_run_receipts (command_id,conversation_id,method,idempotency_key,request_digest,attempt_id,conversation_revision,resume_cursor) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",&[&command,&scope.conversation,&method,&key,&hash,&attempt,&signed,&cursor]).await?;
    Ok(RunReceipt {
        command_id: command,
        attempt_id: attempt.to_owned(),
        conversation_revision: revision,
        resume_cursor: cursor,
    })
}
async fn build_command(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    task: &str,
    attempt: &str,
    pack: &[u8],
) -> Result<TaskCommand, StoreError> {
    let row=tx.query_opt("SELECT a.inputs,a.configuration_revision_id,t.plan_id,t.version_id,t.content_digest,t.approval_id,t.specification,c.instructions,c.provider,c.model,c.max_input_tokens,c.max_output_tokens FROM workflow_task_attempts a JOIN workflow_tasks t ON t.id=a.task_id JOIN workflow_agent_configurations c ON c.workspace_id=a.workspace_id AND c.revision_id=a.configuration_revision_id WHERE a.workspace_id=$1 AND a.project_id=$2 AND a.conversation_id=$3 AND a.task_id=$4 AND a.id=$5",&[&scope.workspace,&scope.project,&scope.conversation,&task,&attempt]).await?.ok_or(StoreError::NotFound)?;
    let i: ExecutionInputs = decode(&row.get::<_, Vec<u8>>(0))?;
    let t: PlanTaskSpecification = decode(&row.get::<_, Vec<u8>>(6))?;
    Ok(TaskCommand {
        attempt_id: attempt.to_owned(),
        task_id: task.to_owned(),
        workspace_id: scope.workspace.to_owned(),
        project_id: scope.project.to_owned(),
        conversation_id: scope.conversation.to_owned(),
        plan_id: row.get(2),
        version_id: row.get(3),
        content_digest: row.get(4),
        approval_id: row.get(5),
        configuration_revision_id: row.get(1),
        generation: 1,
        repository: i.repository,
        source_commit: i.source_commit,
        source_pack_digest: digest(pack),
        lock_digest: i.lock_digest,
        runtime_image: i.runtime_image,
        max_seconds: i.max_seconds,
        max_tokens: i.max_tokens,
        objective: t.objective,
        scope: t.scope,
        out_of_scope: t.out_of_scope,
        acceptance_criteria: t.acceptance_criteria,
        execution_limits: t.execution_limits,
        instructions: row.get(7),
        provider: row.get(8),
        model: row.get(9),
        max_input_tokens: support::unsigned(row.get(10))?,
        max_output_tokens: support::unsigned(row.get(11))?,
    })
}
