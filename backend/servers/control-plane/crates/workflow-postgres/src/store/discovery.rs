use super::{StoreError, WorkflowStore, support};
use crate::model::{Conversation, Project, Workspace};

/// Bounded live discovery page. The continuation is an exclusive ID boundary,
/// not a promise of a frozen catalog snapshot.
#[derive(Debug)]
pub struct DiscoveryPage<T> {
    pub resources: Vec<T>,
    pub next_after_id: String,
}

fn page<T>(mut resources: Vec<T>, limit: usize, id: impl Fn(&T) -> &str) -> DiscoveryPage<T> {
    let next_after_id = if resources.len() > limit {
        resources.truncate(limit);
        resources
            .last()
            .map_or_else(String::new, |value| id(value).to_owned())
    } else {
        String::new()
    };
    DiscoveryPage {
        resources,
        next_after_id,
    }
}

fn bounds(after_id: &str, limit: u32) -> Result<usize, StoreError> {
    if !after_id.is_empty() {
        support::validate_id(after_id, "after_id")?;
    }
    if limit > 100 {
        return Err(StoreError::InvalidInput("limit"));
    }
    Ok(if limit == 0 { 50 } else { limit as usize })
}

impl WorkflowStore {
    /// Discover workspaces in the fixed local operator scope.
    ///
    /// # Errors
    /// Returns invalid bounds, database, or retained-state errors.
    pub async fn list_workspaces(
        &self,
        after_id: &str,
        limit: u32,
    ) -> Result<DiscoveryPage<Workspace>, StoreError> {
        let limit = bounds(after_id, limit)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows = client.query(
            "SELECT id, display_name, workflow_ts_seconds(created_at), workflow_ts_nanos(created_at), \
             workflow_ts_seconds(updated_at), workflow_ts_nanos(updated_at), resource_version \
             FROM workflow_workspaces WHERE id > $1 ORDER BY id LIMIT $2",
            &[&after_id, &i64::try_from(limit + 1).map_err(|_| StoreError::InvalidInput("limit"))?],
        ).await?;
        let resources = rows
            .iter()
            .map(support::workspace_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        driver.abort();
        Ok(page(resources, limit, |value| &value.id))
    }

    /// Discover projects only after validating their workspace ownership.
    ///
    /// # Errors
    /// Returns invalid bounds/scope, missing parent, database, or retained-state errors.
    pub async fn list_projects(
        &self,
        workspace_id: &str,
        after_id: &str,
        limit: u32,
    ) -> Result<DiscoveryPage<Project>, StoreError> {
        support::validate_id(workspace_id, "workspace_id")?;
        let limit = bounds(after_id, limit)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        support::require_workspace(&transaction, workspace_id).await?;
        let rows = transaction
            .query(
                "SELECT id, workspace_id, display_name, goal, workflow_ts_seconds(created_at), \
             workflow_ts_nanos(created_at), workflow_ts_seconds(updated_at), \
             workflow_ts_nanos(updated_at), resource_version FROM workflow_projects \
             WHERE workspace_id = $1 AND id > $2 ORDER BY id LIMIT $3",
                &[
                    &workspace_id,
                    &after_id,
                    &i64::try_from(limit + 1).map_err(|_| StoreError::InvalidInput("limit"))?,
                ],
            )
            .await?;
        let resources = rows
            .iter()
            .map(support::project_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        transaction.commit().await?;
        driver.abort();
        Ok(page(resources, limit, |value| &value.id))
    }

    /// Discover conversations only within their complete parent scope.
    ///
    /// # Errors
    /// Returns invalid bounds/scope, missing parent, database, or retained-state errors.
    pub async fn list_conversations(
        &self,
        workspace_id: &str,
        project_id: &str,
        after_id: &str,
        limit: u32,
    ) -> Result<DiscoveryPage<Conversation>, StoreError> {
        support::validate_id(workspace_id, "workspace_id")?;
        support::validate_id(project_id, "project_id")?;
        let limit = bounds(after_id, limit)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        support::require_project(&transaction, workspace_id, project_id).await?;
        let rows = transaction
            .query(
                "SELECT id, workspace_id, project_id, title, revision, \
             workflow_ts_seconds(created_at), workflow_ts_nanos(created_at) \
             FROM workflow_conversations WHERE workspace_id = $1 AND project_id = $2 \
             AND id > $3 ORDER BY id LIMIT $4",
                &[
                    &workspace_id,
                    &project_id,
                    &after_id,
                    &i64::try_from(limit + 1).map_err(|_| StoreError::InvalidInput("limit"))?,
                ],
            )
            .await?;
        let resources = rows
            .iter()
            .map(support::conversation_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        transaction.commit().await?;
        driver.abort();
        Ok(page(resources, limit, |value| &value.id))
    }
}
