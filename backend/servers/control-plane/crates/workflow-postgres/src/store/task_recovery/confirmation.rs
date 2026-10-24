//! Exact proposal consent and linked dispatch commit together, without reusing a paid-call permit.
use super::{
    RecoveryContent, RecoveryProposal, SELECT, StoreError, WorkflowStore, decode, encode,
    frozen_evidence, projection, support,
};
use crate::{execution::ExecutionInputs, plans::PlanScope};
use ayni_orchestrator_events::task_execution::{TaskCommand, TaskRecoveryCommand, digest};
use serde::{Deserialize, Serialize};
use tokio_postgres::Transaction;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryConfirmation {
    pub command_id: String,
    pub proposal_id: String,
    pub proposal_digest: String,
    pub parent_attempt_id: String,
    pub attempt_id: String,
    pub conversation_revision: u64,
    pub resume_cursor: String,
    pub acknowledged_provider_uncertainty: bool,
}
impl WorkflowStore {
    /// Confirm one exact proposal once, atomically admitting one linked execution segment.
    /// # Errors
    /// Rejects stale evidence/origin, changed receipt keys, missing uncertainty consent and unsupported inputs.
    pub async fn confirm_task_recovery(
        &self,
        scope: PlanScope<'_>,
        task: &str,
        request: &ConfirmRecovery<'_>,
    ) -> Result<RecoveryConfirmation, StoreError> {
        support::validate_scope(scope.workspace, scope.project, scope.conversation)?;
        support::validate_id(task, "task_id")?;
        support::validate_id(request.proposal_id, "proposal_id")?;
        support::validate_key(request.key)?;
        if request.proposal_digest.len() != 71 {
            return Err(StoreError::InvalidInput("proposal_digest"));
        }
        let hash = support::request_digest(
            "ConfirmTaskRecovery",
            &[
                task.as_bytes(),
                request.proposal_id.as_bytes(),
                request.proposal_digest.as_bytes(),
                &[u8::from(request.acknowledge_uncertainty)],
            ],
        );
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        super::super::task_runs::lock(&tx).await?;
        support::conversation(&tx, scope.workspace, scope.project, scope.conversation).await?;
        let row=tx.query_opt(&format!("{SELECT} WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND task_id=$4 AND id=$5 AND kind='proposal'"),&[&scope.workspace,&scope.project,&scope.conversation,&task,&request.proposal_id]).await?.ok_or(StoreError::NotFound)?;
        let record = projection(&row)?;
        if record.content_digest != request.proposal_digest {
            return Err(StoreError::PlanDigestMismatch);
        }
        if let Some(r)=tx.query_opt("SELECT request_digest,result FROM workflow_task_recovery_confirmation_receipts WHERE conversation_id=$1 AND idempotency_key=$2",&[&scope.conversation,&request.key]).await? {
            support::ensure_digest(&r.get::<_,Vec<u8>>(0),&hash)?;
            let result=decode(&r.get::<_,Vec<u8>>(1))?;tx.commit().await?;driver.abort();return Ok(result);
        }
        let result=if let Some(row)=tx.query_opt("SELECT request_digest,result FROM workflow_task_recovery_confirmations WHERE proposal_id=$1",&[&request.proposal_id]).await? {
            support::ensure_digest(&row.get::<_,Vec<u8>>(0),&hash)?;
            decode(&row.get::<_,Vec<u8>>(1))?
        } else {
            admit_proposal(&tx,scope,task,record,request,&hash).await?
        };
        tx.execute("INSERT INTO workflow_task_recovery_confirmation_receipts(conversation_id,idempotency_key,request_digest,proposal_id,result) VALUES($1,$2,$3,$4,$5)",&[&scope.conversation,&request.key,&&hash[..],&request.proposal_id,&encode(&result)?]).await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
}
pub struct ConfirmRecovery<'a> {
    pub proposal_id: &'a str,
    pub proposal_digest: &'a str,
    pub key: &'a str,
    pub acknowledge_uncertainty: bool,
}
async fn eligible(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    task: &str,
    parent: &TaskCommand,
    p: &RecoveryProposal,
    request: &ConfirmRecovery<'_>,
) -> Result<(), StoreError> {
    if p.evidence.provider_uncertain && !request.acknowledge_uncertainty {
        return Err(StoreError::InvalidInput("acknowledge_provider_uncertainty"));
    }
    let row = tx
        .query_one(
            "SELECT specification FROM workflow_tasks WHERE id=$1",
            &[&task],
        )
        .await?;
    let specification: crate::plans::PlanTaskSpecification = decode(&row.get::<_, Vec<u8>>(0))?;
    // Continuation preserves the parent's frozen source and upstream selections;
    // it never substitutes newer dependency work.
    if !specification.unresolved_inputs.is_empty() || specification.repository != parent.repository
    {
        return Err(StoreError::TaskExecutionConflict);
    }
    let latest:String=tx.query_one("SELECT attempt_id FROM workflow_task_runs WHERE task_id=$1 ORDER BY created_at DESC,attempt_id DESC LIMIT 1",&[&task]).await?.get(0);
    if latest != parent.attempt_id {
        return Err(StoreError::TaskExecutionConflict);
    }
    if tx
        .query_opt(
            "SELECT 1 FROM workflow_task_publications WHERE attempt_id=$1",
            &[&parent.attempt_id],
        )
        .await?
        .is_some()
    {
        return Err(StoreError::TaskExecutionConflict);
    }
    if p.request.action == "repair_quality"
        && (p.evidence.quality_state.is_empty() || p.evidence.quality_state == "passed")
    {
        return Err(StoreError::TaskExecutionConflict);
    }
    let count: i64 = tx
        .query_one(
            "SELECT count(*) FROM workflow_task_runs WHERE task_id=$1 OR state='queued'",
            &[&task],
        )
        .await?
        .get(0);
    if count >= 32 {
        return Err(StoreError::Capacity("task_recovery"));
    }
    // Serialize with preparation/observation writers before adding a new preparation projection.
    tx.query_one("SELECT revision FROM workflow_conversations WHERE workspace_id=$1 AND project_id=$2 AND id=$3 FOR UPDATE",&[&scope.workspace,&scope.project,&scope.conversation]).await?;
    Ok(())
}
async fn admit(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    command: &TaskCommand,
    source: &[u8],
    request: &ConfirmRecovery<'_>,
) -> Result<RecoveryConfirmation, StoreError> {
    let recovery = command.recovery.as_ref().ok_or(StoreError::CorruptState)?;
    let command_id = support::new_id();
    let old = tx
        .query_one(
            "SELECT inputs FROM workflow_task_attempts WHERE id=$1",
            &[&recovery.parent_attempt_id],
        )
        .await?;
    let mut inputs: ExecutionInputs = decode(&old.get::<_, Vec<u8>>(0))?;
    inputs.max_seconds = command.max_seconds;
    inputs.max_tokens = command.max_tokens;
    tx.execute("INSERT INTO workflow_task_attempts(id,workspace_id,project_id,conversation_id,plan_id,version_id,content_digest,approval_id,task_id,preparation_command_id,configuration_revision_id,inputs,state) SELECT $1,workspace_id,project_id,conversation_id,plan_id,version_id,content_digest,approval_id,task_id,preparation_command_id,configuration_revision_id,$2,'prepared' FROM workflow_task_attempts WHERE id=$3",&[&command.attempt_id,&encode(&inputs)?,&recovery.parent_attempt_id]).await?;
    tx.execute("INSERT INTO workflow_task_runs(attempt_id,task_id,workspace_id,project_id,conversation_id,command,source_pack,state) VALUES($1,$2,$3,$4,$5,$6,$7,'queued')",&[&command.attempt_id,&command.task_id,&scope.workspace,&scope.project,&scope.conversation,&encode(command)?,&source]).await?;
    let (revision, cursor) = super::super::task_runs::update(
        tx,
        scope,
        &command.task_id,
        &command.attempt_id,
        "task_recovery_confirmed",
    )
    .await?;
    Ok(RecoveryConfirmation {
        command_id,
        proposal_id: request.proposal_id.into(),
        proposal_digest: request.proposal_digest.into(),
        parent_attempt_id: recovery.parent_attempt_id.clone(),
        attempt_id: command.attempt_id.clone(),
        conversation_revision: revision,
        resume_cursor: cursor,
        acknowledged_provider_uncertainty: request.acknowledge_uncertainty,
    })
}

async fn admit_proposal(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    task: &str,
    record: super::RecoveryRecord,
    request: &ConfirmRecovery<'_>,
    hash: &[u8; 32],
) -> Result<RecoveryConfirmation, StoreError> {
    let RecoveryContent::Proposal(proposal) = record.content else {
        return Err(StoreError::CorruptState);
    };
    let parent = super::super::task_runs::read(tx, scope, task, &record.attempt_id).await?;
    let evidence = frozen_evidence(tx, scope, task, &record.attempt_id).await?;
    eligible(tx, scope, task, &parent.command, &proposal, request).await?;
    if evidence != proposal.evidence {
        return Err(StoreError::TaskExecutionConflict);
    }
    let source = tx
        .query_one(
            "SELECT source_pack FROM workflow_task_runs WHERE attempt_id=$1",
            &[&record.attempt_id],
        )
        .await?
        .get::<_, Vec<u8>>(0);
    if source.is_empty() {
        return Err(StoreError::TaskExecutionConflict);
    }
    if digest(&source) != parent.command.source_pack_digest {
        return Err(StoreError::CorruptState);
    }
    let patch = if proposal.request.action == "replace_attempt" {
        String::new()
    } else {
        ayni_orchestrator_events::task_execution::saved_patch(
            &parent.command,
            parent.outcome.as_ref(),
        )
        .to_owned()
    };
    let mut inherited_answers = parent
        .command
        .recovery
        .as_ref()
        .map_or_else(Vec::new, |recovery| recovery.answers.clone());
    inherited_answers.extend(super::worker::answers(tx, &record.attempt_id).await?);
    if inherited_answers.len() > 32 {
        return Err(StoreError::InvalidInput("question_context_limit"));
    }
    let mut command = parent.command;
    command.attempt_id = support::new_id();
    command.generation = 1;
    command.max_seconds = proposal.request.additional_seconds;
    command.max_tokens = proposal.request.additional_tokens;

    command.recovery = Some(TaskRecoveryCommand {
        parent_attempt_id: record.attempt_id.clone(),
        proposal_id: record.id.clone(),
        proposal_digest: record.content_digest.clone(),
        action: proposal.request.action,
        explanation: proposal.request.explanation,
        seed_patch_digest: digest(patch.as_bytes()),
        seed_patch: patch,
        quality_reason: proposal.safe_reason,
        quality_artifact: parent.quality.map(|q| q.artifact).unwrap_or_default(),
        answers: inherited_answers,
    });
    if encode(&command)?.len() > 262_144 {
        return Err(StoreError::InvalidInput("recovery_context_limit"));
    }
    let result = admit(tx, scope, &command, &source, request).await?;
    tx.execute("INSERT INTO workflow_task_recovery_confirmations(proposal_id,parent_attempt_id,attempt_id,request_digest,result) VALUES($1,$2,$3,$4,$5)",&[&request.proposal_id,&record.attempt_id,&result.attempt_id,&hash.as_slice(),&encode(&result)?]).await?;
    Ok(result)
}
