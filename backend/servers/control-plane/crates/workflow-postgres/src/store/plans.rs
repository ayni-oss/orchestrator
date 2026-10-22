mod approval;
pub(super) mod proposals;
mod reads;
use super::{StoreError, WorkflowStore, observation, support};
use crate::plans::{self, PlanContent, PlanScope, PlanWriteReceipt, WrittenPlan};
pub(super) use reads::summaries;
use tokio_postgres::Transaction;

impl WorkflowStore {
    /// Retain one operator plan and initial version, or replay the original result.
    ///
    /// # Errors
    /// Rejects invalid content, scope, capacity, idempotency conflicts or unavailable authority.
    pub async fn create_plan(
        &self,
        scope: PlanScope<'_>,
        key: &str,
        follows: &str,
        content: &PlanContent,
    ) -> Result<WrittenPlan, StoreError> {
        self.write_plan(scope, key, "", "", follows, content).await
    }

    /// Append an immutable version after checking the expected head. Retry precedes the head check.
    ///
    /// # Errors
    /// Rejects stale heads, invalid scope/content, capacity or command conflicts.
    pub async fn revise_plan(
        &self,
        scope: PlanScope<'_>,
        key: &str,
        plan: &str,
        expected: &str,
        content: &PlanContent,
    ) -> Result<WrittenPlan, StoreError> {
        support::validate_id(plan, "plan_id")?;
        support::validate_id(expected, "expected_version_id")?;
        self.write_plan(scope, key, plan, expected, "", content)
            .await
    }

    async fn write_plan(
        &self,
        scope: PlanScope<'_>,
        key: &str,
        plan: &str,
        expected: &str,
        follows: &str,
        content: &PlanContent,
    ) -> Result<WrittenPlan, StoreError> {
        validate_scope(scope)?;
        support::validate_key(key)?;
        if !follows.is_empty() {
            support::validate_id(follows, "follows_plan_id")?;
        }
        let bytes = plans::canonical_content(content)?;
        let method = if plan.is_empty() {
            "CreatePlan"
        } else {
            "RevisePlan"
        };
        let digest = support::request_digest(
            method,
            &[
                scope.workspace.as_bytes(),
                scope.project.as_bytes(),
                scope.conversation.as_bytes(),
                plan.as_bytes(),
                expected.as_bytes(),
                follows.as_bytes(),
                &bytes,
            ],
        );
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let row = tx.query_opt("SELECT revision FROM workflow_conversations WHERE workspace_id=$1 AND project_id=$2 AND id=$3 FOR UPDATE", &[&scope.workspace,&scope.project,&scope.conversation]).await?.ok_or(StoreError::NotFound)?;
        let replay = tx.query_opt("SELECT request_digest,command_id,version_id,conversation_revision,resume_cursor FROM workflow_plan_receipts WHERE conversation_id=$1 AND method=$2 AND idempotency_key=$3", &[&scope.conversation,&method,&key]).await?;
        let result = if let Some(row) = replay {
            support::ensure_digest(&row.get::<_, Vec<u8>>(0), &digest)?;
            let version = reads::version(&tx, scope, "", &row.get::<_, String>(2)).await?;
            WrittenPlan {
                receipt: PlanWriteReceipt {
                    command_id: row.get(1),
                    plan_id: version.plan_id.clone(),
                    version_id: version.id.clone(),
                    conversation_revision: support::unsigned(row.get(3))?,
                    resume_cursor: row.get(4),
                },
                version,
            }
        } else {
            let revision: i64 = row.get(0);
            let revision = revision.checked_add(1).ok_or(StoreError::CorruptState)?;
            let plan_id = if plan.is_empty() {
                support::new_id()
            } else {
                plan.to_owned()
            };
            let version_id = support::new_id();
            let ordinal = prepare_head(
                &tx,
                scope,
                &plan_id,
                plan.is_empty(),
                expected,
                follows,
                &version_id,
            )
            .await?;
            tx.execute("INSERT INTO workflow_plan_versions (id,workspace_id,project_id,conversation_id,plan_id,ordinal,predecessor_id,content,content_digest,title,author) VALUES ($1,$2,$3,$4,$5,$6,NULLIF($7,''),$8,$9,$10,'local-operator')", &[&version_id,&scope.workspace,&scope.project,&scope.conversation,&plan_id,&ordinal,&expected,&bytes,&plans::content_digest(&bytes),&content.title]).await?;
            let cursor = record_update(&tx, scope, revision, &plan_id, &version_id).await?;
            let command = support::new_id();
            tx.execute("INSERT INTO workflow_plan_receipts (conversation_id,method,idempotency_key,request_digest,command_id,version_id,conversation_revision,resume_cursor) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)", &[&scope.conversation,&method,&key,&&digest[..],&command,&version_id,&revision,&cursor]).await?;
            WrittenPlan {
                version: reads::version(&tx, scope, &plan_id, &version_id).await?,
                receipt: PlanWriteReceipt {
                    command_id: command,
                    plan_id,
                    version_id,
                    conversation_revision: support::unsigned(revision)?,
                    resume_cursor: cursor,
                },
            }
        };
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
}

fn validate_scope(scope: PlanScope<'_>) -> Result<(), StoreError> {
    support::validate_scope(scope.workspace, scope.project, scope.conversation)
}

async fn prepare_head(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    plan: &str,
    creating: bool,
    expected: &str,
    follows: &str,
    version: &str,
) -> Result<i64, StoreError> {
    if creating {
        let count: i64 = tx
            .query_one(
                "SELECT count(*) FROM workflow_plans WHERE conversation_id=$1",
                &[&scope.conversation],
            )
            .await?
            .get(0);
        if count >= plans::MAX_PLANS {
            return Err(StoreError::Capacity("conversation_plans"));
        }
        if !follows.is_empty() {
            reads::require_plan(tx, scope, follows).await?;
        }
        tx.execute("INSERT INTO workflow_plans (id,workspace_id,project_id,conversation_id,current_version_id,follows_plan_id) VALUES ($1,$2,$3,$4,$5,NULLIF($6,''))", &[&plan,&scope.workspace,&scope.project,&scope.conversation,&version,&follows]).await?;
        return Ok(1);
    }
    let head = reads::version(tx, scope, plan, "").await?;
    if tx
        .query_opt(
            "SELECT 1 FROM workflow_plan_approvals WHERE plan_id=$1",
            &[&plan],
        )
        .await?
        .is_some()
    {
        return Err(StoreError::PlanAlreadyApproved);
    }
    if head.id != expected {
        return Err(StoreError::PlanVersionConflict);
    }
    if head.ordinal >= u64::try_from(plans::MAX_VERSIONS).map_err(|_| StoreError::CorruptState)? {
        return Err(StoreError::Capacity("plan_versions"));
    }
    tx.execute(
        "UPDATE workflow_plans SET current_version_id=$2 WHERE id=$1",
        &[&plan, &version],
    )
    .await?;
    i64::try_from(head.ordinal + 1).map_err(|_| StoreError::CorruptState)
}

async fn record_update(
    tx: &Transaction<'_>,
    scope: PlanScope<'_>,
    revision: i64,
    plan: &str,
    version: &str,
) -> Result<String, StoreError> {
    tx.execute(
        "UPDATE workflow_conversations SET revision=$2 WHERE id=$1",
        &[&scope.conversation, &revision],
    )
    .await?;
    let activity = support::new_id();
    tx.execute("INSERT INTO workflow_activities (id,workspace_id,project_id,conversation_id,conversation_revision,stage,presentation_state,safe_reason) VALUES ($1,$2,$3,$4,$5,'admission','accepted','plan_version_created')", &[&activity,&scope.workspace,&scope.project,&scope.conversation,&revision]).await?;
    let affected = vec![plan, version];
    let row = tx.query_one("INSERT INTO workflow_updates (id,activity_id,workspace_id,project_id,conversation_id,conversation_revision,kind,affected_resource_ids,presentation_state,safe_reason) VALUES ($1,$2,$3,$4,$5,$6,'plan_version_created',$7,'accepted','plan_version_created') RETURNING cursor", &[&support::new_id(),&activity,&scope.workspace,&scope.project,&scope.conversation,&revision,&affected]).await?;
    observation::notify(tx, scope.conversation).await?;
    Ok(support::cursor(scope.conversation, row.get(0)))
}
