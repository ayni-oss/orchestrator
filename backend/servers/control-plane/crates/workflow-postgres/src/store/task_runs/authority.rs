use super::{
    SELECT_RUN, StoreError, TaskRun, Transaction, WorkflowStore, encode, lock, origin, run_row,
    support, update,
};
use ayni_orchestrator_events::task_execution::{SupervisorRequest, WorkerRequest, WorkerResponse};
impl WorkflowStore {
    /// Poll or reconcile one task using the trusted supervisor identity.
    /// # Errors
    /// Rejects stale workload identity, wrong generation or corrupt retained commands.
    pub async fn task_supervisor(
        &self,
        request: &SupervisorRequest,
    ) -> Result<Option<TaskRun>, StoreError> {
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        lock(&tx).await?;
        let result = match request {
            SupervisorRequest::Poll => poll(&tx).await?,
            SupervisorRequest::Observe {
                attempt_id,
                generation,
                pod_uid,
            } => {
                let run = by_id(&tx, attempt_id).await?;
                require_current(&run, *generation)?;
                // Kubernetes owns Pod UIDs; they are not our UUIDv7 resource IDs.
                let uid = uuid::Uuid::parse_str(pod_uid)
                    .map_err(|_| StoreError::InvalidInput("pod_uid"))?;
                if uid.to_string() != *pod_uid {
                    return Err(StoreError::InvalidInput("pod_uid"));
                }
                if !run.pod_uid.is_empty() && run.pod_uid != *pod_uid {
                    return Err(StoreError::TaskExecutionConflict);
                }
                tx.execute(
                    "UPDATE workflow_task_runs SET pod_uid=$2 WHERE attempt_id=$1",
                    &[&attempt_id, &pod_uid],
                )
                .await?;
                Some(by_id(&tx, attempt_id).await?)
            }
            SupervisorRequest::Stopped {
                attempt_id,
                generation,
                pod_uid,
            } => {
                let run = by_id(&tx, attempt_id).await?;
                if run.command.generation != *generation || run.pod_uid != *pod_uid {
                    return Err(StoreError::TaskExecutionConflict);
                }
                let state = match run.state.as_str() {
                    "cancelling" => "cancelled",
                    "reporting" => run
                        .outcome
                        .as_ref()
                        .ok_or(StoreError::CorruptState)?
                        .state
                        .as_str(),
                    "claimed" => "interrupted",
                    _ => return Ok(Some(run)),
                };
                tx.execute("UPDATE workflow_task_runs SET state=$2,provider_state=CASE WHEN provider_state='sending' THEN 'uncertain' ELSE provider_state END WHERE attempt_id=$1",&[&attempt_id,&state]).await?;
                update(
                    &tx,
                    origin(&run.command),
                    &run.command.task_id,
                    attempt_id,
                    "task_execution_stopped",
                )
                .await?;
                Some(by_id(&tx, attempt_id).await?)
            }
        };
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Worker capability is scoped by the HTTP boundary to one attempt, generation and Pod UID.
    /// # Errors
    /// Rejects fenced, expired or unauthorized work and conflicting result replay.
    pub async fn task_worker(
        &self,
        attempt: &str,
        generation: u64,
        pod_uid: &str,
        request: &WorkerRequest,
    ) -> Result<WorkerResponse, StoreError> {
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        lock(&tx).await?;
        let run = by_id(&tx, attempt).await?;
        if run.generation != generation || run.pod_uid.is_empty() || run.pod_uid != pod_uid {
            return Err(StoreError::TaskExecutionConflict);
        }
        let mut response = WorkerResponse::default();
        match request {
            WorkerRequest::Input => {
                require_current(&run, generation)?;
                within_deadline(&tx, attempt).await?;
                response.command = Some(run.command);
                response.source_pack = tx
                    .query_one(
                        "SELECT source_pack FROM workflow_task_runs WHERE attempt_id=$1",
                        &[&attempt],
                    )
                    .await?
                    .get(0);
            }
            WorkerRequest::Sending { request_digest } => {
                require_current(&run, generation)?;
                if request_digest.len() != 71 || !request_digest.starts_with("sha256:") {
                    return Err(StoreError::InvalidInput("request_digest"));
                }
                response.authorized=tx.execute("UPDATE workflow_task_runs SET provider_state='sending',request_digest=$2 WHERE attempt_id=$1 AND provider_state='not_started' AND deadline_at>clock_timestamp()",&[&attempt,&request_digest]).await?==1;
            }
            WorkerRequest::Report { outcome } => {
                retain_worker_report(&tx, &run, attempt, outcome).await?;
                response.authorized = true;
            }
        }
        tx.commit().await?;
        driver.abort();
        Ok(response)
    }
}
async fn by_id(tx: &Transaction<'_>, attempt: &str) -> Result<TaskRun, StoreError> {
    support::validate_id(attempt, "attempt_id")?;
    let row = tx
        .query_opt(
            &format!("{SELECT_RUN} WHERE attempt_id=$1 FOR UPDATE"),
            &[&attempt],
        )
        .await?
        .ok_or(StoreError::NotFound)?;
    run_row(&row)
}
fn require_current(run: &TaskRun, generation: u64) -> Result<(), StoreError> {
    if run.generation == generation && run.state == "claimed" {
        Ok(())
    } else {
        Err(StoreError::TaskExecutionConflict)
    }
}
async fn poll(tx: &Transaction<'_>) -> Result<Option<TaskRun>, StoreError> {
    if let Some(row)=tx.query_opt(&format!("{SELECT_RUN} WHERE state IN ('claimed','reporting','cancelling') LIMIT 1 FOR UPDATE"),&[]).await?{
  let run=run_row(&row)?;
  if run.state=="claimed" {
   let expired:bool=tx.query_one("SELECT deadline_at<=clock_timestamp() FROM workflow_task_runs WHERE attempt_id=$1",&[&run.command.attempt_id]).await?.get(0);
   if expired{tx.execute("UPDATE workflow_task_runs SET state='cancelling',generation=generation+1,provider_state=CASE WHEN provider_state='sending' THEN 'uncertain' ELSE provider_state END WHERE attempt_id=$1",&[&run.command.attempt_id]).await?;update(tx,origin(&run.command),&run.command.task_id,&run.command.attempt_id,"task_deadline_exceeded").await?;return Ok(Some(by_id(tx,&run.command.attempt_id).await?));}
  }
  return Ok(Some(run));
 }
    let Some(row)=tx.query_opt(&format!("{SELECT_RUN} WHERE state='queued' ORDER BY created_at,attempt_id LIMIT 1 FOR UPDATE"),&[]).await? else{return Ok(None);};
    let run = run_row(&row)?;
    tx.execute("UPDATE workflow_task_runs SET state='claimed',deadline_at=clock_timestamp()+make_interval(secs=>$2) WHERE attempt_id=$1",&[&run.command.attempt_id,&f64::from(run.command.max_seconds)]).await?;
    update(
        tx,
        origin(&run.command),
        &run.command.task_id,
        &run.command.attempt_id,
        "task_claimed",
    )
    .await?;
    Ok(Some(by_id(tx, &run.command.attempt_id).await?))
}

async fn within_deadline(tx: &Transaction<'_>, attempt: &str) -> Result<(), StoreError> {
    let within: bool = tx
        .query_one(
            "SELECT deadline_at>clock_timestamp() FROM workflow_task_runs WHERE attempt_id=$1",
            &[&attempt],
        )
        .await?
        .get(0);
    if within {
        Ok(())
    } else {
        Err(StoreError::TaskExecutionConflict)
    }
}

async fn retain_worker_report(
    tx: &Transaction<'_>,
    run: &TaskRun,
    attempt: &str,
    outcome: &ayni_orchestrator_events::task_execution::TaskOutcome,
) -> Result<(), StoreError> {
    if !outcome.validate() {
        return Err(StoreError::InvalidInput("task_outcome"));
    }
    if let Some(retained) = run.outcome.clone() {
        if retained != *outcome {
            return Err(StoreError::TaskExecutionConflict);
        }
    } else {
        require_current(run, run.generation)?;
        within_deadline(tx, attempt).await?;
        if run.command.provider == "openai"
            && outcome.state == "completed"
            && (run.provider_state != "sending"
                || outcome.request_digest
                    != tx
                        .query_one(
                            "SELECT request_digest FROM workflow_task_runs WHERE attempt_id=$1",
                            &[&attempt],
                        )
                        .await?
                        .get::<_, String>(0))
        {
            return Err(StoreError::TaskExecutionConflict);
        }
        let provider = if outcome.state == "uncertain" {
            "uncertain"
        } else {
            "settled"
        };
        tx.execute("UPDATE workflow_task_runs SET state='reporting',outcome=$2,provider_state=$3 WHERE attempt_id=$1",&[&attempt,&encode(outcome)?,&provider]).await?;
        update(
            tx,
            origin(&run.command),
            &run.command.task_id,
            attempt,
            "task_result_retained",
        )
        .await?;
    }
    Ok(())
}
