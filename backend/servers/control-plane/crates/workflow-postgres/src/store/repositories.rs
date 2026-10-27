//! Shared source catalog. Immutable receipts never assert runtime readiness.
use super::{
    StoreError, WorkflowStore, connections, new_id, require_project, validate_id, validate_key,
};
use serde::{Deserialize, Serialize};
use tokio_postgres::{Row, Transaction};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Repository {
    pub id: String,
    pub connection_id: String,
    pub installation_id: u64,
    pub provider_repository_id: u64,
    pub full_name: String,
    pub default_branch: String,
    pub private: bool,
    pub connected_at_unix_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub idempotency_key: String,
    pub connection_id: String,
    pub connection_revision: u64,
    pub installation_id: u64,
    pub provider_repository_id: u64,
    pub state: String,
    pub safe_reason: String,
    pub repository: Option<Repository>,
    pub accepted_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}
#[derive(Clone, Serialize)]
pub struct Connect<'a> {
    pub key: &'a str,
    pub connection_id: &'a str,
    pub expected_revision: u64,
    pub installation_id: u64,
    pub provider_repository_id: u64,
}
// Credential-bearing data is intentionally neither Debug nor Serialize.
pub struct SourceCredential {
    pub app_id: String,
    pub private_key_pem: String,
}
#[derive(Clone, Debug)]
pub struct ObservedRepository {
    pub provider_repository_id: u64,
    pub full_name: String,
    pub default_branch: String,
    pub private: bool,
}
fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::InvalidInput("provider identity"))
}
fn positive(value: u64) -> Result<i64, StoreError> {
    if value == 0 {
        return Err(StoreError::InvalidInput("provider identity"));
    }
    integer(value)
}
fn unsigned(row: &Row, name: &str) -> Result<u64, StoreError> {
    u64::try_from(row.get::<_, i64>(name)).map_err(|_| StoreError::CorruptState)
}
fn repository(row: &Row) -> Result<Repository, StoreError> {
    Ok(Repository {
        id: row.get("id"),
        connection_id: row.get("connection_id"),
        installation_id: unsigned(row, "installation_id")?,
        provider_repository_id: unsigned(row, "provider_repository_id")?,
        full_name: row.get("full_name"),
        default_branch: row.get("default_branch"),
        private: row.get("private"),
        connected_at_unix_ms: unsigned(row, "connected_at_unix_ms")?,
    })
}
fn operation(row: &Row) -> Result<Operation, StoreError> {
    let result: String = row.get("result");
    Ok(Operation {
        id: row.get("id"),
        idempotency_key: row.get("idempotency_key"),
        connection_id: row.get("connection_id"),
        connection_revision: unsigned(row, "connection_revision")?,
        installation_id: unsigned(row, "installation_id")?,
        provider_repository_id: unsigned(row, "provider_repository_id")?,
        state: row.get("state"),
        safe_reason: row.get("safe_reason"),
        repository: if result.is_empty() {
            None
        } else {
            Some(serde_json::from_str(&result).map_err(|_| StoreError::CorruptState)?)
        },
        accepted_at_unix_ms: unsigned(row, "accepted_at_unix_ms")?,
        updated_at_unix_ms: unsigned(row, "updated_at_unix_ms")?,
    })
}
fn page_bounds(after: &str, limit: u32) -> Result<i64, StoreError> {
    if !after.is_empty() {
        validate_id(after, "after_id")?;
    }
    if limit > 100 {
        return Err(StoreError::InvalidInput("limit"));
    }
    Ok(i64::from(if limit == 0 { 50 } else { limit }))
}
fn page<T>(mut values: Vec<T>, limit: i64, id: impl Fn(&T) -> &str) -> (Vec<T>, String) {
    let next = if values.len() > usize::try_from(limit).unwrap_or(0) {
        values.pop();
        values.last().map_or_else(String::new, |v| id(v).to_owned())
    } else {
        String::new()
    };
    (values, next)
}
fn validate_observation(observed: &Result<ObservedRepository, &str>) -> Result<(), StoreError> {
    if let Ok(value) = observed {
        if value.full_name.len() > 256
            || value.full_name.split('/').count() != 2
            || value.full_name.split('/').any(str::is_empty)
            || value.full_name.chars().any(char::is_control)
            || value.default_branch.is_empty()
            || value.default_branch.len() > 256
            || value.default_branch.chars().any(char::is_control)
        {
            return Err(StoreError::InvalidInput("repository observation"));
        }
    } else if !matches!(
        observed,
        Err("github_key_invalid"
            | "github_authentication_failed"
            | "github_access_denied"
            | "github_rate_limited"
            | "github_unavailable"
            | "github_response_invalid"
            | "github_repository_unavailable"
            | "github_source_changed")
    ) {
        return Err(StoreError::InvalidInput("repository reason"));
    }
    Ok(())
}
async fn retain_repository(
    tx: &Transaction<'_>,
    op: &Operation,
    value: ObservedRepository,
) -> Result<(&'static str, &'static str, String), StoreError> {
    if value.provider_repository_id != op.provider_repository_id {
        return Err(StoreError::InvalidInput("repository identity"));
    }
    let provider = positive(value.provider_repository_id)?;
    let installation = positive(op.installation_id)?;
    tx.execute("INSERT INTO workflow_repositories (id,connection_id,installation_id,provider_repository_id,full_name,default_branch,private) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(provider_repository_id) DO NOTHING",&[&new_id(),&op.connection_id,&installation,&provider,&value.full_name,&value.default_branch,&value.private]).await?;
    let repo = repository(
        &tx.query_one(
            "SELECT * FROM workflow_repositories WHERE provider_repository_id=$1",
            &[&provider],
        )
        .await?,
    )?;
    if repo.connection_id != op.connection_id || repo.installation_id != op.installation_id {
        return Ok(("failed", "repository_already_connected", String::new()));
    }
    Ok((
        "completed",
        "",
        serde_json::to_string(&repo).map_err(|_| StoreError::CorruptState)?,
    ))
}

impl WorkflowStore {
    /// Load only the exact enabled active source credential under its connection lock.
    /// # Errors
    /// Reject invalid IDs, disabled/unvalidated sources, stale revisions and database errors.
    pub async fn repository_source(
        &self,
        id: &str,
        expected: u64,
    ) -> Result<SourceCredential, StoreError> {
        validate_id(id, "connection_id")?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let c = connections::locked(&tx, id).await?;
        if !c.enabled
            || c.provider != "github"
            || c.capability != "source_control"
            || c.revision != expected
            || c.active_credential_revision.is_empty()
        {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        let row=tx.query_one("SELECT value FROM workflow_connection_credentials WHERE connection_id=$1 AND revision_id=$2",&[&id,&c.active_credential_revision]).await?;
        let result = SourceCredential {
            app_id: c.app_id,
            private_key_pem: row.get(0),
        };
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Freeze intent before GitHub access; exact retries recover one accepted operation.
    /// # Errors
    /// Reject invalid inputs, conflicting keys, unavailable sources and database errors.
    pub async fn begin_repository_connect(
        &self,
        input: Connect<'_>,
    ) -> Result<Operation, StoreError> {
        validate_key(input.key)?;
        validate_id(input.connection_id, "connection_id")?;
        let installation = positive(input.installation_id)?;
        let provider = positive(input.provider_repository_id)?;
        let expected = positive(input.expected_revision)?;
        let safe = serde_json::to_string(&input).map_err(|_| StoreError::CorruptState)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, input.key).await?;
        if let Some(row) = tx
            .query_opt(
                "SELECT * FROM workflow_repository_operations WHERE idempotency_key=$1",
                &[&input.key],
            )
            .await?
        {
            if row.get::<_, String>("safe_request") != safe {
                return Err(StoreError::IdempotencyConflict);
            }
            let result = operation(&row)?;
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let c = connections::locked(&tx, input.connection_id).await?;
        if !c.enabled
            || c.provider != "github"
            || c.capability != "source_control"
            || c.revision != input.expected_revision
            || c.active_credential_revision.is_empty()
        {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        let row=tx.query_one("INSERT INTO workflow_repository_operations (id,idempotency_key,safe_request,connection_id,connection_revision,credential_revision,installation_id,provider_repository_id,state) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'accepted') RETURNING *",&[&new_id(),&input.key,&safe,&input.connection_id,&expected,&c.active_credential_revision,&installation,&provider]).await?;
        let result = operation(&row)?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Accept only reports for the frozen source generation; source rotations fence late results.
    /// # Errors
    /// Reject malformed observations, unknown operations and database/retained-state errors.
    pub async fn finish_repository_connect(
        &self,
        key: &str,
        observed: Result<ObservedRepository, &str>,
    ) -> Result<Operation, StoreError> {
        validate_key(key)?;
        validate_observation(&observed)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let original = tx
            .query_opt(
                "SELECT connection_id FROM workflow_repository_operations WHERE idempotency_key=$1",
                &[&key],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let c = connections::locked(&tx, original.get("connection_id")).await?;
        let row = tx
            .query_one(
                "SELECT * FROM workflow_repository_operations WHERE idempotency_key=$1 FOR UPDATE",
                &[&key],
            )
            .await?;
        let op = operation(&row)?;
        if op.state != "accepted" {
            tx.commit().await?;
            driver.abort();
            return Ok(op);
        }
        let mut result = String::new();
        let (state, reason) = if !c.enabled
            || c.revision != op.connection_revision
            || c.active_credential_revision != row.get::<_, String>("credential_revision")
        {
            ("superseded", "github_source_changed")
        } else {
            match observed {
                Err(reason) => (
                    if matches!(reason, "github_unavailable" | "github_rate_limited") {
                        "accepted"
                    } else {
                        "failed"
                    },
                    reason,
                ),
                Ok(value) => {
                    let accepted = retain_repository(&tx, &op, value).await?;
                    result = accepted.2;
                    (accepted.0, accepted.1)
                }
            }
        };
        let row=tx.query_one("UPDATE workflow_repository_operations SET state=$2,safe_reason=$3,result=$4,updated_at_unix_ms=(extract(epoch FROM clock_timestamp())*1000)::bigint WHERE idempotency_key=$1 RETURNING *",&[&key,&state,&reason,&result]).await?;
        let result = operation(&row)?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Read a retained safe receipt without resuming work.
    /// # Errors
    /// Reject malformed/unknown keys or database errors.
    pub async fn get_repository_operation(&self, key: &str) -> Result<Operation, StoreError> {
        validate_key(key)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client
            .query_opt(
                "SELECT * FROM workflow_repository_operations WHERE idempotency_key=$1",
                &[&key],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let result = operation(&row)?;
        driver.abort();
        Ok(result)
    }
    /// List retained source operations in bounded ID order.
    /// # Errors
    /// Reject invalid bounds and database/retained-state errors.
    pub async fn list_repository_operations(
        &self,
        after: &str,
        limit: u32,
    ) -> Result<(Vec<Operation>, String), StoreError> {
        let limit = page_bounds(after, limit)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows = client
            .query(
                "SELECT * FROM workflow_repository_operations WHERE ($1='' OR id<$1) ORDER BY id DESC LIMIT $2",
                &[&after, &(limit + 1)],
            )
            .await?;
        let values = rows.iter().map(operation).collect::<Result<Vec<_>, _>>()?;
        driver.abort();
        Ok(page(values, limit, |v| &v.id))
    }
    /// List shared catalog entries or associations after checking the complete project scope.
    /// # Errors
    /// Reject invalid scope/bounds, missing project and database errors.
    pub async fn list_repositories(
        &self,
        scope: Option<(&str, &str)>,
        after: &str,
        limit: u32,
    ) -> Result<(Vec<Repository>, String), StoreError> {
        let limit = page_bounds(after, limit)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let rows = if let Some((workspace, project)) = scope {
            validate_id(workspace, "workspace_id")?;
            validate_id(project, "project_id")?;
            require_project(&tx, workspace, project).await?;
            tx.query("SELECT r.* FROM workflow_repositories r JOIN workflow_project_repositories a ON a.repository_id=r.id WHERE a.workspace_id=$1 AND a.project_id=$2 AND r.id>$3 ORDER BY r.id LIMIT $4",&[&workspace,&project,&after,&(limit+1)]).await?
        } else {
            tx.query(
                "SELECT * FROM workflow_repositories WHERE id>$1 ORDER BY id LIMIT $2",
                &[&after, &(limit + 1)],
            )
            .await?
        };
        let values = rows.iter().map(repository).collect::<Result<Vec<_>, _>>()?;
        tx.commit().await?;
        driver.abort();
        Ok(page(values, limit, |v| &v.id))
    }
    /// Associate a catalog repository directly with one project; never confer integration administration.
    /// # Errors
    /// Reject invalid scope, missing resources, conflicting retries and unavailable source connections.
    pub async fn associate_repository(
        &self,
        key: &str,
        workspace: &str,
        project: &str,
        repo: &str,
    ) -> Result<(Repository, String), StoreError> {
        validate_key(key)?;
        for (value, field) in [
            (workspace, "workspace_id"),
            (project, "project_id"),
            (repo, "repository_id"),
        ] {
            validate_id(value, field)?;
        }
        let safe = serde_json::to_string(&(workspace, project, repo))
            .map_err(|_| StoreError::CorruptState)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, key).await?;
        require_project(&tx, workspace, project).await?;
        let resource = repository(
            &tx.query_opt("SELECT * FROM workflow_repositories WHERE id=$1", &[&repo])
                .await?
                .ok_or(StoreError::NotFound)?,
        )?;
        if let Some(row)=tx.query_opt("SELECT command_id,safe_request FROM workflow_repository_association_receipts WHERE idempotency_key=$1",&[&key]).await? {
            if row.get::<_,String>("safe_request")!=safe {return Err(StoreError::IdempotencyConflict);}
            let command=row.get(0);tx.commit().await?;driver.abort();return Ok((resource,command));
        }
        let c = connections::locked(&tx, &resource.connection_id).await?;
        if !c.enabled || c.active_credential_revision.is_empty() {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        tx.execute("INSERT INTO workflow_project_repositories (workspace_id,project_id,repository_id) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",&[&workspace,&project,&repo]).await?;
        let command = new_id();
        tx.execute("INSERT INTO workflow_repository_association_receipts (idempotency_key,command_id,safe_request,repository_id) VALUES ($1,$2,$3,$4)",&[&key,&command,&safe,&repo]).await?;
        tx.commit().await?;
        driver.abort();
        Ok((resource, command))
    }
}
