//! Optimistically fenced hierarchy edits with immutable exact-retry receipts.
use super::{
    StoreError, WorkflowStore, connections, new_id, require_project, require_workspace,
    support::validate_text, validate_id, validate_key, validate_name,
};
enum Edit<'a> {
    Name(&'a str),
    Goal(&'a str),
}

impl WorkflowStore {
    /// Rename a workspace once under its expected resource version.
    /// # Errors
    /// Reject invalid names/scope, stale versions, conflicting keys and database failures.
    pub async fn rename_workspace(
        &self,
        key: &str,
        workspace: &str,
        expected: u64,
        name: &str,
    ) -> Result<String, StoreError> {
        self.edit_resource(key, workspace, None, expected, Edit::Name(name))
            .await
    }
    /// Rename a project after verifying its workspace ownership.
    /// # Errors
    /// Reject invalid names/scope, stale versions, conflicting keys and database failures.
    pub async fn rename_project(
        &self,
        key: &str,
        workspace: &str,
        project: &str,
        expected: u64,
        name: &str,
    ) -> Result<String, StoreError> {
        self.edit_resource(key, workspace, Some(project), expected, Edit::Name(name))
            .await
    }
    /// Update the broader project objective under its expected resource version.
    /// # Errors
    /// Reject invalid goals or scope, stale versions, conflicting keys and database failures.
    pub async fn set_project_goal(
        &self,
        key: &str,
        workspace: &str,
        project: &str,
        expected: u64,
        goal: &str,
    ) -> Result<String, StoreError> {
        self.edit_resource(key, workspace, Some(project), expected, Edit::Goal(goal))
            .await
    }
    async fn edit_resource(
        &self,
        key: &str,
        workspace: &str,
        project: Option<&str>,
        expected: u64,
        edit: Edit<'_>,
    ) -> Result<String, StoreError> {
        validate_key(key)?;
        validate_id(workspace, "workspace_id")?;
        let version = i64::try_from(expected)
            .ok()
            .filter(|version| *version > 0)
            .ok_or(StoreError::InvalidInput("resource_version"))?;
        let safe = match edit {
            Edit::Name(name) => {
                validate_name(name, "display_name")?;
                if name.len() > 120 {
                    return Err(StoreError::InvalidInput("rename"));
                }
                // Keep the exact retained representation for preexisting rename receipts.
                serde_json::to_string(&(workspace, project, expected, name))
            }
            Edit::Goal(goal) => {
                validate_text(goal, 16_384, true, "goal")?;
                serde_json::to_string(&("SetProjectGoal", workspace, project, expected, goal))
            }
        }
        .map_err(|_| StoreError::CorruptState)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, key).await?;
        if let Some(project) = project {
            validate_id(project, "project_id")?;
            require_project(&tx, workspace, project).await?;
        } else {
            require_workspace(&tx, workspace).await?;
        }
        if let Some(row)=tx.query_opt("SELECT command_id,safe_request FROM workflow_hierarchy_rename_receipts WHERE idempotency_key=$1",&[&key]).await? {
            if row.get::<_,String>("safe_request")!=safe {return Err(StoreError::IdempotencyConflict);}
            let command=row.get(0);tx.commit().await?;driver.abort();return Ok(command);
        }
        let changed = match (project, edit) {
            (Some(project), Edit::Name(name)) => tx.execute(
                "UPDATE workflow_projects SET display_name=$3,resource_version=resource_version+1,updated_at=clock_timestamp() WHERE workspace_id=$1 AND id=$2 AND resource_version=$4",
                &[&workspace,&project,&name,&version],
            ).await?,
            (None, Edit::Name(name)) => tx.execute(
                "UPDATE workflow_workspaces SET display_name=$2,resource_version=resource_version+1,updated_at=clock_timestamp() WHERE id=$1 AND resource_version=$3",
                &[&workspace,&name,&version],
            ).await?,
            (Some(project), Edit::Goal(goal)) => tx.execute(
                "UPDATE workflow_projects SET goal=$3,resource_version=resource_version+1,updated_at=clock_timestamp() WHERE workspace_id=$1 AND id=$2 AND resource_version=$4",
                &[&workspace,&project,&goal,&version],
            ).await?,
            (None, Edit::Goal(_)) => return Err(StoreError::InvalidInput("project_id")),
        };
        if changed != 1 {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        let command = new_id();
        tx.execute("INSERT INTO workflow_hierarchy_rename_receipts (idempotency_key,command_id,safe_request,workspace_id,project_id) VALUES ($1,$2,$3,$4,$5)",&[&key,&command,&safe,&workspace,&project]).await?;
        tx.commit().await?;
        driver.abort();
        Ok(command)
    }
}
