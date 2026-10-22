//! Explicit operator authorization; this module creates no dispatch intent.
use super::{StoreError, WorkflowStore, reads, validate_scope};
use crate::{
    plans::{AdmittedTask, ApprovedPlan, PlanApprovalReceipt, PlanScope, PlanVersion, TaskPage},
    store::{observation, support},
};
use tokio_postgres::{GenericClient, IsolationLevel, Row};

impl WorkflowStore {
    /// Atomically approve an exact current version and admit its complete task set.
    ///
    /// # Errors
    /// Rejects stale reviews, digest/scope conflicts, incomplete scope or second approval.
    pub async fn approve_plan(
        &self,
        scope: PlanScope<'_>,
        key: &str,
        plan: &str,
        expected: &str,
        digest: &str,
    ) -> Result<ApprovedPlan, StoreError> {
        validate_scope(scope)?;
        support::validate_key(key)?;
        support::validate_id(plan, "plan_id")?;
        support::validate_id(expected, "expected_version_id")?;
        validate_digest(digest)?;
        let request = support::request_digest(
            "ApprovePlan",
            &[
                scope.workspace.as_bytes(),
                scope.project.as_bytes(),
                scope.conversation.as_bytes(),
                plan.as_bytes(),
                expected.as_bytes(),
                digest.as_bytes(),
            ],
        );
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let row = tx.query_opt("SELECT revision FROM workflow_conversations WHERE workspace_id=$1 AND project_id=$2 AND id=$3 FOR UPDATE", &[&scope.workspace,&scope.project,&scope.conversation]).await?.ok_or(StoreError::NotFound)?;
        // Exact replay precedes every mutable precondition, including approved state.
        if let Some(row) = tx.query_opt("SELECT request_digest,plan_id FROM workflow_plan_approvals WHERE conversation_id=$1 AND idempotency_key=$2", &[&scope.conversation,&key]).await? {
            support::ensure_digest(&row.get::<_, Vec<u8>>(0), &request)?;
            let result = approved(&tx, scope, &row.get::<_, String>(1)).await?.ok_or(StoreError::CorruptState)?;
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let version = reads::version(&tx, scope, plan, "").await?;
        if approved(&tx, scope, plan).await?.is_some() {
            return Err(StoreError::PlanAlreadyApproved);
        }
        require_reviewed_scope(&version, expected, digest)?;
        let revision = row
            .get::<_, i64>(0)
            .checked_add(1)
            .ok_or(StoreError::CorruptState)?;
        let approval = support::new_id();
        let command = support::new_id();
        let tasks: Vec<String> = version
            .content
            .tasks
            .iter()
            .map(|_| support::new_id())
            .collect();
        tx.execute(
            "UPDATE workflow_conversations SET revision=$2 WHERE id=$1",
            &[&scope.conversation, &revision],
        )
        .await?;
        let activity = support::new_id();
        tx.execute("INSERT INTO workflow_activities (id,workspace_id,project_id,conversation_id,conversation_revision,stage,presentation_state,waiting_on,safe_reason) VALUES ($1,$2,$3,$4,$5,'admission','accepted','execution_capability','plan_approved')", &[&activity,&scope.workspace,&scope.project,&scope.conversation,&revision]).await?;
        let mut affected = vec![plan.to_owned(), expected.to_owned(), approval.clone()];
        affected.extend(tasks.iter().cloned());
        let update = tx.query_one("INSERT INTO workflow_updates (id,activity_id,workspace_id,project_id,conversation_id,conversation_revision,kind,affected_resource_ids,presentation_state,waiting_on,safe_reason) VALUES ($1,$2,$3,$4,$5,$6,'plan_approved',$7,'accepted','execution_capability','plan_approved') RETURNING cursor", &[&support::new_id(),&activity,&scope.workspace,&scope.project,&scope.conversation,&revision,&affected]).await?;
        let cursor = support::cursor(scope.conversation, update.get(0));
        tx.execute("INSERT INTO workflow_plan_approvals (id,command_id,workspace_id,project_id,conversation_id,plan_id,version_id,content_digest,operator,idempotency_key,request_digest,task_ids,conversation_revision,resume_cursor) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'local-operator',$9,$10,$11,$12,$13)", &[&approval,&command,&scope.workspace,&scope.project,&scope.conversation,&plan,&expected,&digest,&key,&&request[..],&tasks,&revision,&cursor]).await?;
        retain_tasks(&tx, scope, &version, &approval, &tasks).await?;
        observation::notify(&tx, scope.conversation).await?;
        let result = approved(&tx, scope, plan)
            .await?
            .ok_or(StoreError::CorruptState)?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }

    /// Recover the original receipt and ordered task set without granting authority.
    ///
    /// # Errors
    /// Returns validation, missing scope or retained-state errors.
    pub async fn get_plan_approval(
        &self,
        scope: PlanScope<'_>,
        plan: &str,
    ) -> Result<Option<ApprovedPlan>, StoreError> {
        validate_scope(scope)?;
        support::validate_id(plan, "plan_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        reads::require_plan(&client, scope, plan).await?;
        let result = approved(&client, scope, plan).await?;
        driver.abort();
        Ok(result)
    }

    /// Read one immutable admitted task inside its full originating scope.
    ///
    /// # Errors
    /// Returns validation, missing scope or retained-state errors.
    pub async fn get_task(
        &self,
        scope: PlanScope<'_>,
        task: &str,
    ) -> Result<AdmittedTask, StoreError> {
        validate_scope(scope)?;
        support::validate_id(task, "task_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client.query_opt(&format!("{TASK_SELECT} WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND id=$4"), &[&scope.workspace,&scope.project,&scope.conversation,&task]).await?.ok_or(StoreError::NotFound)?;
        let result = task_row(&row)?;
        driver.abort();
        Ok(result)
    }

    /// Bounded stable admission listing; immutable task IDs are an exclusive cursor.
    ///
    /// # Errors
    /// Returns validation, missing owner scope or database errors.
    pub async fn list_tasks(
        &self,
        scope: PlanScope<'_>,
        after: &str,
        limit: u32,
    ) -> Result<TaskPage, StoreError> {
        validate_scope(scope)?;
        if !after.is_empty() {
            support::validate_id(after, "after_id")?;
        }
        if limit > 32 {
            return Err(StoreError::InvalidInput("task_list_limit"));
        }
        let limit = if limit == 0 { 20 } else { limit };
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .start()
            .await?;
        support::conversation(&tx, scope.workspace, scope.project, scope.conversation).await?;
        if !after.is_empty() {
            tx.query_opt("SELECT 1 FROM workflow_tasks WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND id=$4", &[&scope.workspace,&scope.project,&scope.conversation,&after]).await?.ok_or(StoreError::InvalidInput("after_id"))?;
        }
        let rows = tx
            .query(
                &format!("{TASK_SELECT} WHERE conversation_id=$1 AND id>$2 ORDER BY id LIMIT $3"),
                &[&scope.conversation, &after, &i64::from(limit + 1)],
            )
            .await?;
        let tasks = rows
            .iter()
            .take(limit as usize)
            .map(task_row)
            .collect::<Result<Vec<_>, _>>()?;
        let next_after_id = if rows.len() > limit as usize {
            tasks.last().ok_or(StoreError::CorruptState)?.id.clone()
        } else {
            String::new()
        };
        tx.commit().await?;
        driver.abort();
        Ok(TaskPage {
            tasks,
            next_after_id,
        })
    }
}

const TASK_SELECT: &str = "SELECT id,workspace_id,project_id,conversation_id,plan_id,version_id,content_digest,approval_id,proposed_task_key,specification,state,execution_blockers,workflow_ts_seconds(created_at),workflow_ts_nanos(created_at) FROM workflow_tasks";

fn task_row(row: &Row) -> Result<AdmittedTask, StoreError> {
    let bytes: Vec<u8> = row.get(9);
    let specification = serde_json::from_slice(&bytes).map_err(|_| StoreError::CorruptState)?;
    Ok(AdmittedTask {
        id: row.get(0),
        workspace_id: row.get(1),
        project_id: row.get(2),
        conversation_id: row.get(3),
        plan_id: row.get(4),
        version_id: row.get(5),
        content_digest: row.get(6),
        approval_id: row.get(7),
        proposed_task_key: row.get(8),
        specification,
        state: row.get(10),
        execution_blockers: row.get(11),
        created_at: support::timestamp(row.get(12), row.get(13))?,
    })
}

async fn approved(
    client: &(impl GenericClient + Sync),
    scope: PlanScope<'_>,
    plan: &str,
) -> Result<Option<ApprovedPlan>, StoreError> {
    let Some(row) = client.query_opt("SELECT id,command_id,workspace_id,project_id,conversation_id,plan_id,version_id,content_digest,operator,request_digest,workflow_ts_seconds(created_at),workflow_ts_nanos(created_at),conversation_revision,resume_cursor,task_ids FROM workflow_plan_approvals WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND plan_id=$4", &[&scope.workspace,&scope.project,&scope.conversation,&plan]).await? else { return Ok(None); };
    let digest: Vec<u8> = row.get(9);
    let mut request_digest = String::from("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(request_digest, "{byte:02x}").map_err(|_| StoreError::CorruptState)?;
    }
    let approval = PlanApprovalReceipt {
        id: row.get(0),
        command_id: row.get(1),
        workspace_id: row.get(2),
        project_id: row.get(3),
        conversation_id: row.get(4),
        plan_id: row.get(5),
        version_id: row.get(6),
        content_digest: row.get(7),
        operator: row.get(8),
        request_digest,
        created_at: support::timestamp(row.get(10), row.get(11))?,
        conversation_revision: support::unsigned(row.get(12))?,
        resume_cursor: row.get(13),
        task_ids: row.get(14),
    };
    let rows = client
        .query(
            &format!("{TASK_SELECT} WHERE approval_id=$1 ORDER BY array_position($2::text[],id)"),
            &[&approval.id, &approval.task_ids],
        )
        .await?;
    let tasks = rows.iter().map(task_row).collect::<Result<Vec<_>, _>>()?;
    if tasks
        .iter()
        .map(|task| &task.id)
        .ne(approval.task_ids.iter())
    {
        return Err(StoreError::CorruptState);
    }
    Ok(Some(ApprovedPlan { approval, tasks }))
}

fn validate_digest(value: &str) -> Result<(), StoreError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(StoreError::InvalidInput("content_digest"));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(StoreError::InvalidInput("content_digest"));
    }
    Ok(())
}

fn require_reviewed_scope(
    version: &PlanVersion,
    expected: &str,
    digest: &str,
) -> Result<(), StoreError> {
    if version.id != expected {
        return Err(StoreError::PlanVersionConflict);
    }
    if version.content_digest != digest {
        return Err(StoreError::PlanDigestMismatch);
    }
    if !version.content.unresolved_questions.is_empty()
        || version.content.tasks.is_empty()
        || version
            .content
            .tasks
            .iter()
            .any(|task| task.acceptance_criteria.is_empty())
    {
        return Err(StoreError::PlanUnresolvedScope);
    }
    Ok(())
}

async fn retain_tasks(
    tx: &tokio_postgres::Transaction<'_>,
    scope: PlanScope<'_>,
    version: &PlanVersion,
    approval: &str,
    tasks: &[String],
) -> Result<(), StoreError> {
    for (id, task) in tasks.iter().zip(&version.content.tasks) {
        let specification = serde_json::to_vec(task).map_err(|_| StoreError::CorruptState)?;
        let mut blockers = vec![
            "Repository and source must be verified and frozen before execution".to_owned(),
            "An execution runtime must be selected".to_owned(),
            "An execution budget must be authorized".to_owned(),
        ];
        blockers.extend(task.unresolved_inputs.iter().cloned());
        tx.execute("INSERT INTO workflow_tasks (id,workspace_id,project_id,conversation_id,plan_id,version_id,content_digest,approval_id,proposed_task_key,specification,state,execution_blockers) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'awaiting_execution',$11", &[id,&scope.workspace,&scope.project,&scope.conversation,&version.plan_id,&version.id,&version.content_digest,&approval,&task.key,&specification,&blockers]).await?;
    }
    Ok(())
}
