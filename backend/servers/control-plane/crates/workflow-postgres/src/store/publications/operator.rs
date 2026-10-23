use super::{
    PlanScope, PublicationCommand, PublicationReceipt, SELECT, StoreError, Transaction,
    WorkflowStore, decode, encode, lock, row, support, update,
};
use crate::execution::ExecutionInputs;
use ayni_orchestrator_events::{
    task_execution::{TaskCommand, TaskOutcome, digest},
    task_publication::{base_branch, github_repository},
    task_quality::{QualityOutcome, snapshot_digest},
};
impl WorkflowStore {
    /// Authorize one immutable publication after reviewing exact changes and quality.
    /// # Errors
    /// Rejects wrong scope, changed review evidence, conflicting receipts or ineligible attempts.
    pub async fn start_task_publication(
        &self,
        s: PlanScope<'_>,
        task: &str,
        attempt: &str,
        key: &str,
        patch: &str,
        quality: &str,
    ) -> Result<PublicationReceipt, StoreError> {
        validate(s, task, attempt, key)?;
        let hash = support::request_digest(
            "StartTaskPublication",
            &[
                task.as_bytes(),
                attempt.as_bytes(),
                patch.as_bytes(),
                quality.as_bytes(),
            ],
        );
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        lock(&tx).await?;
        support::conversation(&tx, s.workspace, s.project, s.conversation).await?;
        if let Some(result) = receipt(&tx, s, "StartTaskPublication", key, &hash).await? {
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let command = build_command(&tx, s, task, attempt, patch, quality).await?;
        if tx
            .query_opt(
                "SELECT 1 FROM workflow_task_publications WHERE attempt_id=$1",
                &[&attempt],
            )
            .await?
            .is_some()
        {
            return Err(StoreError::TaskExecutionConflict);
        }
        let count: i64 = tx
            .query_one(
                "SELECT count(*) FROM workflow_task_publications WHERE state='queued'",
                &[],
            )
            .await?
            .get(0);
        if count >= 32 {
            return Err(StoreError::Capacity("publication_queue"));
        }
        tx.execute("INSERT INTO workflow_task_publications(publication_id,attempt_id,task_id,workspace_id,project_id,conversation_id,repository,command,state) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'queued')",&[&command.publication_id,&attempt,&task,&s.workspace,&s.project,&s.conversation,&command.repository,&encode(&command)?]).await?;
        let result = retain(
            &tx,
            &command,
            "StartTaskPublication",
            key,
            &hash,
            "publication_queued",
        )
        .await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Stop future draft creation before its once-only send permit is consumed.
    /// # Errors
    /// Rejects terminal or possibly externally-created drafts; cancellation can retain a pushed branch.
    pub async fn cancel_task_publication(
        &self,
        s: PlanScope<'_>,
        task: &str,
        attempt: &str,
        key: &str,
    ) -> Result<PublicationReceipt, StoreError> {
        validate(s, task, attempt, key)?;
        let hash = support::request_digest(
            "CancelTaskPublication",
            &[task.as_bytes(), attempt.as_bytes()],
        );
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        lock(&tx).await?;
        support::conversation(&tx, s.workspace, s.project, s.conversation).await?;
        if let Some(result) = receipt(&tx, s, "CancelTaskPublication", key, &hash).await? {
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let run=row(&tx.query_opt(&format!("{SELECT} WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND task_id=$4 AND attempt_id=$5 FOR UPDATE"),&[&s.workspace,&s.project,&s.conversation,&task,&attempt]).await?.ok_or(StoreError::NotFound)?)?;
        if !matches!(run.state.as_str(), "queued" | "claimed" | "prepared") {
            return Err(StoreError::TaskExecutionConflict);
        }
        tx.execute("UPDATE workflow_task_publications SET state='cancelled',generation=generation+1 WHERE publication_id=$1",&[&run.command.publication_id]).await?;
        let result = retain(
            &tx,
            &run.command,
            "CancelTaskPublication",
            key,
            &hash,
            "publication_cancelled_branch_may_exist",
        )
        .await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
}
fn validate(s: PlanScope<'_>, task: &str, attempt: &str, key: &str) -> Result<(), StoreError> {
    support::validate_scope(s.workspace, s.project, s.conversation)?;
    support::validate_id(task, "task_id")?;
    support::validate_id(attempt, "attempt_id")?;
    support::validate_key(key)
}
async fn receipt(
    tx: &Transaction<'_>,
    s: PlanScope<'_>,
    method: &str,
    key: &str,
    hash: &[u8; 32],
) -> Result<Option<PublicationReceipt>, StoreError> {
    tx.query_opt("SELECT request_digest,result FROM workflow_task_publication_receipts WHERE conversation_id=$1 AND method=$2 AND idempotency_key=$3",&[&s.conversation,&method,&key]).await?.map(|r|{support::ensure_digest(&r.get::<_,Vec<u8>>(0),hash)?;decode(&r.get::<_,Vec<u8>>(1))}).transpose()
}
async fn retain(
    tx: &Transaction<'_>,
    command: &PublicationCommand,
    method: &str,
    key: &str,
    hash: &[u8],
    reason: &str,
) -> Result<PublicationReceipt, StoreError> {
    let (revision, cursor) = update(tx, command, reason).await?;
    let result = PublicationReceipt {
        command_id: support::new_id(),
        publication_id: command.publication_id.clone(),
        attempt_id: command.attempt_id.clone(),
        conversation_revision: revision,
        resume_cursor: cursor,
    };
    tx.execute("INSERT INTO workflow_task_publication_receipts(command_id,conversation_id,method,idempotency_key,request_digest,publication_id,result) VALUES($1,$2,$3,$4,$5,$6,$7)",&[&result.command_id,&command.conversation_id,&method,&key,&hash,&command.publication_id,&encode(&result)?]).await?;
    Ok(result)
}
async fn build_command(
    tx: &Transaction<'_>,
    s: PlanScope<'_>,
    task: &str,
    attempt: &str,
    patch_digest: &str,
    quality_digest: &str,
) -> Result<PublicationCommand, StoreError> {
    let row=tx.query_opt("SELECT r.command,r.state,r.outcome,q.outcome,a.inputs,workflow_ts_seconds(clock_timestamp()) FROM workflow_task_runs r JOIN workflow_task_quality q ON q.attempt_id=r.attempt_id JOIN workflow_task_attempts a ON a.id=r.attempt_id WHERE r.workspace_id=$1 AND r.project_id=$2 AND r.conversation_id=$3 AND r.task_id=$4 AND r.attempt_id=$5 FOR UPDATE OF r",&[&s.workspace,&s.project,&s.conversation,&task,&attempt]).await?.ok_or(StoreError::NotFound)?;
    if tx
        .query_opt(
            "SELECT 1 FROM workflow_task_recovery_confirmations WHERE parent_attempt_id=$1",
            &[&attempt],
        )
        .await?
        .is_some()
    {
        return Err(StoreError::TaskExecutionConflict);
    }
    let coding: TaskCommand = decode(&row.get::<_, Vec<u8>>(0))?;
    let outcome: TaskOutcome = decode(
        &row.get::<_, Option<Vec<u8>>>(2)
            .ok_or(StoreError::TaskExecutionConflict)?,
    )?;
    let quality_bytes: Vec<u8> = row.get(3);
    let quality: QualityOutcome = decode(&quality_bytes)?;
    if row.get::<_, String>(1) != "completed"
        || outcome.state != "completed"
        || outcome.patch.is_empty()
        || !outcome.validate()
        || quality.state != "passed"
        || !quality.validate()
        || quality.snapshot_digest != snapshot_digest(&coding, &outcome)
        || patch_digest != digest(outcome.patch.as_bytes())
        || quality_digest != digest(&quality_bytes)
    {
        return Err(StoreError::TaskExecutionConflict);
    }
    let inputs: ExecutionInputs = decode(&row.get::<_, Vec<u8>>(4))?;
    let repository = github_repository(&inputs.repository)
        .ok_or(StoreError::InvalidInput("publication_repository"))?;
    let base_branch =
        base_branch(&inputs.source_ref).ok_or(StoreError::InvalidInput("publication_base"))?;
    let publication_id = attempt.to_owned();
    let title = ayni_orchestrator_events::task_publication::title(&coding.objective);
    let author_name = "Ayni Orchestrator".to_owned();
    let author_email = "orchestrator@ayni.build".to_owned();
    let commit_message =
        format!("{title}\n\nSigned-off-by: {author_name} <{author_email}>\nai-generated: true\n");
    let body = ayni_orchestrator_events::task_publication::body(
        &coding,
        quality_digest,
        patch_digest,
        &publication_id,
    );
    if body.len() > 24 * 1024 {
        return Err(StoreError::InvalidInput("publication_metadata"));
    }
    Ok(PublicationCommand {
        publication_id,
        attempt_id: attempt.into(),
        task_id: task.into(),
        workspace_id: s.workspace.into(),
        project_id: s.project.into(),
        conversation_id: s.conversation.into(),
        generation: 1,
        repository,
        base_branch,
        branch: format!("task/{attempt}"),
        source_commit: coding.source_commit,
        source_pack_digest: coding.source_pack_digest,
        lock_digest: coding.lock_digest,
        patch: outcome.patch,
        quality_digest: quality_digest.into(),
        patch_digest: patch_digest.into(),
        title,
        body,
        author_name,
        author_email,
        commit_message,
        commit_timestamp: row.get(5),
        deadline_seconds: row.get::<_, i64>(5) + 600,
    })
}
