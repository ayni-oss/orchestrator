//! Once-only publication admission and fenced external action authority.
mod authority;
mod operator;
use super::{StoreError, WorkflowStore, observation, support};
use crate::plans::PlanScope;
use ayni_orchestrator_events::task_publication::{PublicationCommand, PublicationRun};
use serde::{Deserialize, Serialize};
use tokio_postgres::{Row, Transaction};
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublicationReceipt {
    pub command_id: String,
    pub publication_id: String,
    pub attempt_id: String,
    pub conversation_revision: u64,
    pub resume_cursor: String,
}
const SELECT: &str = "SELECT command,state,generation,pod_uid,COALESCE(workflow_ts_seconds(lease_until),0),commit_sha,tree_sha,outcome FROM workflow_task_publications";
async fn lock(tx: &Transaction<'_>) -> Result<(), StoreError> {
    tx.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&(crate::migration_lock_id() + 5)],
    )
    .await?;
    Ok(())
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(value).map_err(|_| StoreError::CorruptState)
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(bytes).map_err(|_| StoreError::CorruptState)
}
fn row(row: &Row) -> Result<PublicationRun, StoreError> {
    Ok(PublicationRun {
        command: decode(&row.get::<_, Vec<u8>>(0))?,
        state: row.get(1),
        generation: support::unsigned(row.get(2))?,
        pod_uid: row.get(3),
        lease_seconds: row.get(4),
        commit_sha: row.get(5),
        tree_sha: row.get(6),
        outcome: row
            .get::<_, Option<Vec<u8>>>(7)
            .map(|b| decode(&b))
            .transpose()?,
    })
}
async fn by_id(tx: &Transaction<'_>, id: &str) -> Result<PublicationRun, StoreError> {
    support::validate_id(id, "publication_id")?;
    row(&tx
        .query_opt(
            &format!("{SELECT} WHERE publication_id=$1 FOR UPDATE"),
            &[&id],
        )
        .await?
        .ok_or(StoreError::NotFound)?)
}
fn scope(command: &PublicationCommand) -> PlanScope<'_> {
    PlanScope {
        workspace: &command.workspace_id,
        project: &command.project_id,
        conversation: &command.conversation_id,
    }
}
async fn update(
    tx: &Transaction<'_>,
    command: &PublicationCommand,
    reason: &str,
) -> Result<(u64, String), StoreError> {
    let s = scope(command);
    let rev = tx
        .query_one(
            "SELECT revision FROM workflow_conversations WHERE id=$1 FOR UPDATE",
            &[&s.conversation],
        )
        .await?
        .get::<_, i64>(0)
        .checked_add(1)
        .ok_or(StoreError::CorruptState)?;
    tx.execute(
        "UPDATE workflow_conversations SET revision=$2 WHERE id=$1",
        &[&s.conversation, &rev],
    )
    .await?;
    let activity = support::new_id();
    tx.execute("INSERT INTO workflow_activities(id,workspace_id,project_id,conversation_id,conversation_revision,stage,presentation_state,safe_reason) VALUES($1,$2,$3,$4,$5,'execution','accepted',$6)",&[&activity,&s.workspace,&s.project,&s.conversation,&rev,&reason]).await?;
    let ids = vec![
        command.task_id.as_str(),
        command.attempt_id.as_str(),
        command.publication_id.as_str(),
    ];
    let cursor=tx.query_one("INSERT INTO workflow_updates(id,activity_id,workspace_id,project_id,conversation_id,conversation_revision,kind,affected_resource_ids,presentation_state,safe_reason) VALUES($1,$2,$3,$4,$5,$6,'task_execution_changed',$7,'accepted',$8) RETURNING cursor",&[&support::new_id(),&activity,&s.workspace,&s.project,&s.conversation,&rev,&ids,&reason]).await?.get(0);
    observation::notify(tx, s.conversation).await?;
    Ok((
        support::unsigned(rev)?,
        support::cursor(s.conversation, cursor),
    ))
}
impl WorkflowStore {
    /// Read publication only through its exact task origin.
    /// # Errors
    /// Rejects missing task scope or corrupt retained state.
    pub async fn get_task_publication(
        &self,
        s: PlanScope<'_>,
        task: &str,
        attempt: &str,
    ) -> Result<Option<PublicationRun>, StoreError> {
        self.get_task_run(s, task, attempt).await?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let result=client.query_opt(&format!("{SELECT} WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND task_id=$4 AND attempt_id=$5"),&[&s.workspace,&s.project,&s.conversation,&task,&attempt]).await?.map(|r|row(&r)).transpose();
        driver.abort();
        result
    }
}
