//! Optimistically fenced hierarchy edits with immutable exact-retry receipts.
use super::{
    StoreError, WorkflowStore, connections, new_id, require_project, require_workspace,
    validate_id, validate_key, validate_name,
};
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
        self.rename_resource(key, workspace, None, expected, name)
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
        self.rename_resource(key, workspace, Some(project), expected, name)
            .await
    }
    async fn rename_resource(
        &self,
        key: &str,
        workspace: &str,
        project: Option<&str>,
        expected: u64,
        name: &str,
    ) -> Result<String, StoreError> {
        validate_key(key)?;
        validate_id(workspace, "workspace_id")?;
        validate_name(name, "display_name")?;
        if name.len() > 120 || expected == 0 {
            return Err(StoreError::InvalidInput("rename"));
        }
        let version =
            i64::try_from(expected).map_err(|_| StoreError::InvalidInput("resource_version"))?;
        let safe = serde_json::to_string(&(workspace, project, expected, name))
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
        let changed = if let Some(project) = project {
            tx.execute("UPDATE workflow_projects SET display_name=$3,resource_version=resource_version+1,updated_at=clock_timestamp() WHERE workspace_id=$1 AND id=$2 AND resource_version=$4",&[&workspace,&project,&name,&version]).await?
        } else {
            tx.execute("UPDATE workflow_workspaces SET display_name=$2,resource_version=resource_version+1,updated_at=clock_timestamp() WHERE id=$1 AND resource_version=$3",&[&workspace,&name,&version]).await?
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
