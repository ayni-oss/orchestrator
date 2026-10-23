use super::{
    PublicationRun, SELECT, StoreError, Transaction, WorkflowStore, by_id, encode, lock, row,
    update,
};
use ayni_orchestrator_events::task_publication::{
    PublicationOutcome, PublicationSupervisorRequest, PublicationWorkerRequest,
    PublicationWorkerResponse, oid,
};
impl WorkflowStore {
    /// Claim or recover publication work only within the configured repository binding.
    /// # Errors
    /// Rejects foreign scope and invalid workload identity.
    pub async fn publication_supervisor(
        &self,
        workspace: &str,
        repository: &str,
        request: &PublicationSupervisorRequest,
    ) -> Result<Option<PublicationRun>, StoreError> {
        let PublicationSupervisorRequest::Poll {
            workspace_id,
            repository: requested,
            pod_uid,
        } = request;
        if workspace_id != workspace || requested != repository {
            return Err(StoreError::TaskExecutionConflict);
        }
        validate_uid(pod_uid)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        lock(&tx).await?;
        let result = poll(&tx, workspace, repository, pod_uid).await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Apply one fenced publisher step; a consumed create permit can never be reset.
    /// # Errors
    /// Rejects expired ownership, conflicting commits/results and invalid state transitions.
    pub async fn publication_worker(
        &self,
        binding: (&str, &str),
        id: &str,
        generation: u64,
        uid: &str,
        request: &PublicationWorkerRequest,
    ) -> Result<PublicationWorkerResponse, StoreError> {
        let (workspace, repository) = binding;
        validate_uid(uid)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        lock(&tx).await?;
        let run = by_id(&tx, id).await?;
        require_owner(&run, (workspace, repository), generation, uid)?;
        let mut result = PublicationWorkerResponse::default();
        if let PublicationWorkerRequest::Report { outcome } = request
            && run.outcome.as_ref() == Some(outcome)
            && matches!(run.state.as_str(), "draft" | "blocked")
        {
            result.authorized = true;
            result.run = Some(run);
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        require_active(&tx, &run).await?;
        if expire(&tx, &run).await? {
            result.run = Some(by_id(&tx, id).await?);
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        match request {
            PublicationWorkerRequest::Input => {
                result.command = Some(run.command.clone());
                result.source_pack = tx
                    .query_one(
                        "SELECT source_pack FROM workflow_task_runs WHERE attempt_id=$1",
                        &[&run.command.attempt_id],
                    )
                    .await?
                    .get(0);
                result.authorized = true;
            }
            PublicationWorkerRequest::Prepared {
                commit_sha,
                tree_sha,
            } => {
                prepared(&tx, &run, commit_sha, tree_sha).await?;
                result.authorized = true;
            }
            PublicationWorkerRequest::BeginCreate => {
                result.authorized=tx.execute("UPDATE workflow_task_publications SET state='creating' WHERE publication_id=$1 AND state='prepared'",&[&id]).await?==1;
                if !matches!(run.state.as_str(), "prepared" | "creating" | "uncertain") {
                    return Err(StoreError::TaskExecutionConflict);
                }
            }
            PublicationWorkerRequest::Report { outcome } => {
                report(&tx, &run, outcome).await?;
                result.authorized = true;
            }
        }
        renew(&tx, id).await?;
        result.run = Some(by_id(&tx, id).await?);
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
}
fn validate_uid(uid: &str) -> Result<(), StoreError> {
    let parsed = uuid::Uuid::parse_str(uid).map_err(|_| StoreError::InvalidInput("pod_uid"))?;
    if parsed.to_string() != uid {
        return Err(StoreError::InvalidInput("pod_uid"));
    }
    Ok(())
}
async fn renew(tx: &Transaction<'_>, id: &str) -> Result<(), StoreError> {
    tx.execute("UPDATE workflow_task_publications SET lease_until=clock_timestamp()+interval '90 seconds' WHERE publication_id=$1",&[&id]).await?;
    Ok(())
}
async fn require_active(tx: &Transaction<'_>, run: &PublicationRun) -> Result<(), StoreError> {
    if !matches!(
        run.state.as_str(),
        "claimed" | "prepared" | "creating" | "uncertain"
    ) {
        return Err(StoreError::TaskExecutionConflict);
    }
    let current:bool=tx.query_one("SELECT lease_until>clock_timestamp() FROM workflow_task_publications WHERE publication_id=$1",&[&run.command.publication_id]).await?.get(0);
    if !current {
        return Err(StoreError::TaskExecutionConflict);
    }
    Ok(())
}
async fn poll(
    tx: &Transaction<'_>,
    workspace: &str,
    repository: &str,
    uid: &str,
) -> Result<Option<PublicationRun>, StoreError> {
    let active=tx.query_opt(&format!("{SELECT} WHERE workspace_id=$1 AND repository=$2 AND state IN ('claimed','prepared','creating','uncertain') LIMIT 1 FOR UPDATE"),&[&workspace,&repository]).await?;
    if let Some(r) = active {
        let run = row(&r)?;
        if expire(tx, &run).await? {
            return Ok(None);
        }
        let valid:bool=tx.query_one("SELECT lease_until>clock_timestamp() FROM workflow_task_publications WHERE publication_id=$1",&[&run.command.publication_id]).await?.get(0);
        if run.pod_uid != uid && valid {
            return Ok(None);
        }
        if run.pod_uid != uid {
            tx.execute("UPDATE workflow_task_publications SET generation=generation+1,pod_uid=$2,state=CASE WHEN state='creating' THEN 'uncertain' ELSE state END WHERE publication_id=$1",&[&run.command.publication_id,&uid]).await?;
        }
        renew(tx, &run.command.publication_id).await?;
        return Ok(Some(by_id(tx, &run.command.publication_id).await?));
    }
    let pending=tx.query_opt(&format!("{SELECT} WHERE workspace_id=$1 AND repository=$2 AND state='queued' ORDER BY created_at,publication_id LIMIT 1 FOR UPDATE"),&[&workspace,&repository]).await?;
    let Some(r) = pending else {
        return Ok(None);
    };
    let run = row(&r)?;
    if expire(tx, &run).await? {
        return Ok(None);
    }
    tx.execute(
        "UPDATE workflow_task_publications SET state='claimed',pod_uid=$2 WHERE publication_id=$1",
        &[&run.command.publication_id, &uid],
    )
    .await?;
    renew(tx, &run.command.publication_id).await?;
    update(tx, &run.command, "publication_claimed").await?;
    Ok(Some(by_id(tx, &run.command.publication_id).await?))
}
async fn prepared(
    tx: &Transaction<'_>,
    run: &PublicationRun,
    commit: &str,
    tree: &str,
) -> Result<(), StoreError> {
    if !oid(commit) || !oid(tree) {
        return Err(StoreError::InvalidInput("publication_commit"));
    }
    if !run.commit_sha.is_empty() {
        if run.commit_sha != commit || run.tree_sha != tree {
            return Err(StoreError::TaskExecutionConflict);
        }
        return Ok(());
    }
    if run.state != "claimed" {
        return Err(StoreError::TaskExecutionConflict);
    }
    tx.execute("UPDATE workflow_task_publications SET state='prepared',commit_sha=$2,tree_sha=$3 WHERE publication_id=$1",&[&run.command.publication_id,&commit,&tree]).await?;
    update(tx, &run.command, "publication_commit_prepared").await?;
    Ok(())
}
async fn report(
    tx: &Transaction<'_>,
    run: &PublicationRun,
    outcome: &PublicationOutcome,
) -> Result<(), StoreError> {
    if !outcome.validate(&run.command, &run.commit_sha) {
        return Err(StoreError::InvalidInput("publication_outcome"));
    }
    if matches!(outcome.state.as_str(), "draft" | "uncertain")
        && !matches!(run.state.as_str(), "creating" | "uncertain")
    {
        return Err(StoreError::TaskExecutionConflict);
    }
    if outcome.state == "blocked" && matches!(run.state.as_str(), "creating" | "uncertain") {
        return Err(StoreError::TaskExecutionConflict);
    }
    if let Some(retained) = &run.outcome {
        if retained == outcome {
            return Ok(());
        }
        if retained.state != "uncertain" || outcome.state != "draft" {
            return Err(StoreError::TaskExecutionConflict);
        }
    }
    retain_history(tx, run, outcome).await?;
    tx.execute(
        "UPDATE workflow_task_publications SET state=$2,outcome=$3 WHERE publication_id=$1",
        &[
            &run.command.publication_id,
            &outcome.state,
            &encode(outcome)?,
        ],
    )
    .await?;
    update(tx, &run.command, "publication_result_retained").await?;
    Ok(())
}

async fn expire(tx: &Transaction<'_>, run: &PublicationRun) -> Result<bool, StoreError> {
    if !matches!(run.state.as_str(), "queued" | "claimed" | "prepared") {
        return Ok(false);
    }
    let now: i64 = tx
        .query_one("SELECT workflow_ts_seconds(clock_timestamp())", &[])
        .await?
        .get(0);
    if now <= run.command.deadline_seconds {
        return Ok(false);
    }
    let outcome = PublicationOutcome {
        state: "blocked".into(),
        safe_reason: "publication_expired".into(),
        commit_sha: run.commit_sha.clone(),
        ..Default::default()
    };
    retain_history(tx, run, &outcome).await?;
    tx.execute(
        "UPDATE workflow_task_publications SET state='blocked',outcome=$2 WHERE publication_id=$1",
        &[&run.command.publication_id, &encode(&outcome)?],
    )
    .await?;
    update(tx, &run.command, "publication_expired").await?;
    Ok(true)
}

fn require_owner(
    run: &PublicationRun,
    binding: (&str, &str),
    generation: u64,
    uid: &str,
) -> Result<(), StoreError> {
    let (workspace, repository) = binding;
    if run.command.workspace_id != workspace
        || run.command.repository != repository
        || run.generation != generation
        || run.pod_uid != uid
    {
        return Err(StoreError::TaskExecutionConflict);
    }
    Ok(())
}

async fn retain_history(
    tx: &Transaction<'_>,
    run: &PublicationRun,
    outcome: &PublicationOutcome,
) -> Result<(), StoreError> {
    let bytes = encode(outcome)?;
    let hash = ayni_orchestrator_events::task_execution::digest(&bytes);
    let count: i64 = tx
        .query_one(
            "SELECT count(*) FROM workflow_task_publication_reports WHERE publication_id=$1",
            &[&run.command.publication_id],
        )
        .await?
        .get(0);
    if count >= 2 {
        return Err(StoreError::TaskExecutionConflict);
    }
    let ordinal = i16::try_from(count + 1).map_err(|_| StoreError::CorruptState)?;
    let generation = i64::try_from(run.generation).map_err(|_| StoreError::CorruptState)?;
    tx.execute("INSERT INTO workflow_task_publication_reports(publication_id,ordinal,report_digest,outcome,generation,pod_uid) VALUES($1,$2,$3,$4,$5,$6)",&[&run.command.publication_id,&ordinal,&hash,&bytes,&generation,&run.pod_uid]).await?;
    Ok(())
}
