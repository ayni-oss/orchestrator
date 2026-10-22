use super::{StoreError, WorkflowStore, validate_scope};
use crate::{
    model::Timestamp,
    plans::{
        self, AgentPlanProvenance, PlanScope, PlanSummary, PlanVersion, VersionPage, VersionSummary,
    },
    store::support,
};
use tokio_postgres::{GenericClient, Row};

pub(super) async fn require_plan(
    client: &(impl GenericClient + Sync),
    scope: PlanScope<'_>,
    plan: &str,
) -> Result<(), StoreError> {
    client.query_opt("SELECT 1 FROM workflow_plans WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND id=$4", &[&scope.workspace,&scope.project,&scope.conversation,&plan]).await?.ok_or(StoreError::NotFound)?;
    Ok(())
}

pub(super) async fn version(
    client: &(impl GenericClient + Sync),
    scope: PlanScope<'_>,
    plan: &str,
    version: &str,
) -> Result<PlanVersion, StoreError> {
    let row = client.query_opt("SELECT v.id,v.plan_id,v.workspace_id,v.project_id,v.conversation_id,v.ordinal,COALESCE(v.predecessor_id,''),v.content_digest,v.content,v.author,workflow_ts_seconds(v.created_at),workflow_ts_nanos(v.created_at) FROM workflow_plan_versions v JOIN workflow_plans p ON p.id=v.plan_id WHERE v.workspace_id=$1 AND v.project_id=$2 AND v.conversation_id=$3 AND ($4='' OR v.plan_id=$4) AND v.id=CASE WHEN $5='' THEN p.current_version_id ELSE $5 END", &[&scope.workspace,&scope.project,&scope.conversation,&plan,&version]).await?.ok_or(StoreError::NotFound)?;
    let bytes: Vec<u8> = row.get(8);
    let content = serde_json::from_slice(&bytes).map_err(|_| StoreError::CorruptState)?;
    let digest: String = row.get(7);
    if plans::canonical_content(&content)? != bytes || plans::content_digest(&bytes) != digest {
        return Err(StoreError::CorruptState);
    }
    Ok(PlanVersion {
        id: row.get(0),
        plan_id: row.get(1),
        workspace_id: row.get(2),
        project_id: row.get(3),
        conversation_id: row.get(4),
        ordinal: support::unsigned(row.get(5))?,
        predecessor_id: row.get(6),
        content_digest: digest,
        content,
        author: row.get(9),
        created_at: stamp(&row, 10)?,
        provenance: provenance(client, &row.get::<_, String>(0)).await?,
    })
}

async fn provenance(
    client: &(impl GenericClient + Sync),
    version: &str,
) -> Result<Option<AgentPlanProvenance>, StoreError> {
    client.query_opt("SELECT source_turn_id,configuration_revision_id,context_digest,provider_call_id,provider_response_id,assignment_id,executor_generation,report_source,report_id,report_digest,invocation_id,assumptions,evidence FROM workflow_plan_proposals WHERE version_id=$1", &[&version]).await?.map(|row| Ok(AgentPlanProvenance {
        source_turn_id: row.get(0), configuration_revision_id: row.get(1),
        context_digest: row.get(2), provider_call_id: row.get(3), provider_response_id: row.get(4),
        assignment_id: row.get(5), executor_generation: support::unsigned(row.get(6))?,
        report_source: row.get(7), report_id: row.get(8), report_digest: row.get(9),
        invocation_id: row.get(10), assumptions: row.get(11), evidence: row.get(12),
    })).transpose()
}

fn stamp(row: &Row, at: usize) -> Result<Timestamp, StoreError> {
    support::timestamp(row.get(at), row.get(at + 1))
}

pub async fn summaries(
    client: &(impl GenericClient + Sync),
    conversation: &str,
) -> Result<Vec<PlanSummary>, StoreError> {
    client.query("SELECT p.id,p.workspace_id,p.project_id,p.conversation_id,p.current_version_id,v.ordinal,v.title,COALESCE(p.follows_plan_id,'') FROM workflow_plans p JOIN workflow_plan_versions v ON v.id=p.current_version_id WHERE p.conversation_id=$1 ORDER BY p.id LIMIT 32", &[&conversation]).await?.into_iter().map(|r| Ok(PlanSummary { id:r.get(0),workspace_id:r.get(1),project_id:r.get(2),conversation_id:r.get(3),current_version_id:r.get(4),current_ordinal:support::unsigned(r.get(5))?,title:r.get(6),follows_plan_id:r.get(7) })).collect()
}

impl WorkflowStore {
    /// Read the current or a specific immutable version inside the complete owner scope.
    ///
    /// # Errors
    /// Returns scope, validation, retained-state or database errors.
    pub async fn get_plan_version(
        &self,
        scope: PlanScope<'_>,
        plan: &str,
        version_id: &str,
    ) -> Result<PlanVersion, StoreError> {
        validate_scope(scope)?;
        support::validate_id(plan, "plan_id")?;
        if !version_id.is_empty() {
            support::validate_id(version_id, "version_id")?;
        }
        let (client, driver) = crate::connect(&self.database_url).await?;
        let result = version(&client, scope, plan, version_id).await?;
        driver.abort();
        Ok(result)
    }

    /// List the bounded plan heads of one conversation, without model-context sharing.
    ///
    /// # Errors
    /// Returns validation, missing scope or database errors.
    pub async fn list_plans(&self, scope: PlanScope<'_>) -> Result<Vec<PlanSummary>, StoreError> {
        validate_scope(scope)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        support::conversation(&tx, scope.workspace, scope.project, scope.conversation).await?;
        let result = summaries(&tx, scope.conversation).await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }

    /// Descending immutable version history. A nonzero next ordinal is an exclusive cursor.
    ///
    /// # Errors
    /// Returns validation, missing scope or database errors.
    pub async fn list_plan_versions(
        &self,
        scope: PlanScope<'_>,
        plan: &str,
        before: u64,
        limit: u32,
    ) -> Result<VersionPage, StoreError> {
        validate_scope(scope)?;
        support::validate_id(plan, "plan_id")?;
        if limit > 20 || before > 65 {
            return Err(StoreError::InvalidInput("plan_history_limit"));
        }
        let limit = i64::from(if limit == 0 { 10 } else { limit });
        let before =
            i64::try_from(before).map_err(|_| StoreError::InvalidInput("before_ordinal"))?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        require_plan(&client, scope, plan).await?;
        let rows=client.query("SELECT id,ordinal,content_digest,author,workflow_ts_seconds(created_at),workflow_ts_nanos(created_at) FROM workflow_plan_versions WHERE plan_id=$1 AND ($2::bigint=0 OR ordinal<$2) ORDER BY ordinal DESC LIMIT $3", &[&plan,&before,&(limit+1)]).await?;
        let has_more = rows.len() > usize::try_from(limit).map_err(|_| StoreError::CorruptState)?;
        let versions = rows
            .iter()
            .take(usize::try_from(limit).map_err(|_| StoreError::CorruptState)?)
            .map(|r| {
                Ok(VersionSummary {
                    id: r.get(0),
                    ordinal: support::unsigned(r.get(1))?,
                    content_digest: r.get(2),
                    author: r.get(3),
                    created_at: stamp(r, 4)?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let next_before_ordinal = if has_more {
            versions.last().ok_or(StoreError::CorruptState)?.ordinal
        } else {
            0
        };
        driver.abort();
        Ok(VersionPage {
            versions,
            next_before_ordinal,
        })
    }
}
