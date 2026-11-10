//! Integration lifecycle. Credentials never enter safe receipts or projections.
use super::{StoreError, WorkflowStore, new_id, validate_id, validate_key, validate_name};
use serde::{Deserialize, Serialize};
mod providers;
pub(crate) use providers::registry_namespace;
pub use providers::{ProviderCandidate, SaveProvider};
use tokio_postgres::{Client, Row, Transaction};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub capability: String,
    pub revision: u64,
    pub enabled: bool,
    pub app_id: String,
    pub active_credential_revision: String,
    pub candidate_credential_revision: String,
    pub status: String,
    pub safe_reason: String,
    #[serde(default)]
    pub settings: ProviderSettings,
    #[serde(default)]
    pub models: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub connection_id: String,
    pub kind: String,
    pub state: String,
    pub safe_reason: String,
    pub connection_revision: u64,
    pub connection: Connection,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderSettings {
    #[serde(default)]
    pub registry_destination: String,
    #[serde(default)]
    pub push_username: String,
    #[serde(default)]
    pub pull_username: String,
}
// Deliberately neither Debug nor Serialize.
pub struct GithubCandidate {
    pub operation: Operation,
    pub app_id: String,
    pub private_key_pem: Option<String>,
}
enum StateChange {
    SetEnabled(bool),
    Delete,
}
// Write-only input; deliberately neither Debug nor Serialize.
pub struct SaveGithub<'a> {
    pub key: &'a str,
    pub id: &'a str,
    pub expected: u64,
    pub name: &'a str,
    pub app_id: &'a str,
    pub private_key_pem: &'a str,
}

fn connection(row: &Row) -> Result<Connection, StoreError> {
    Ok(Connection {
        id: row.get("id"),
        display_name: row.get("display_name"),
        provider: row.get("provider"),
        capability: row.get("capability"),
        revision: u64::try_from(row.get::<_, i64>("revision"))
            .map_err(|_| StoreError::CorruptState)?,
        enabled: row.get("enabled"),
        app_id: row.get("app_id"),
        active_credential_revision: row.get("active_credential_revision"),
        candidate_credential_revision: row.get("candidate_credential_revision"),
        status: row.get("status"),
        safe_reason: row.get("safe_reason"),
        settings: serde_json::from_str(row.get("settings"))
            .map_err(|_| StoreError::CorruptState)?,
        models: serde_json::from_str(row.get("models")).map_err(|_| StoreError::CorruptState)?,
    })
}
fn operation(row: &Row) -> Result<Operation, StoreError> {
    Ok(Operation {
        id: row.get("id"),
        connection_id: row.get("connection_id"),
        kind: row.get("kind"),
        state: row.get("state"),
        safe_reason: row.get("safe_reason"),
        connection_revision: u64::try_from(row.get::<_, i64>("connection_revision"))
            .map_err(|_| StoreError::CorruptState)?,
        connection: serde_json::from_str(row.get("result"))
            .map_err(|_| StoreError::CorruptState)?,
    })
}
pub(super) async fn locked(tx: &Transaction<'_>, id: &str) -> Result<Connection, StoreError> {
    connection(
        &tx.query_opt(
            "SELECT * FROM workflow_connections WHERE id=$1 FOR UPDATE",
            &[&id],
        )
        .await?
        .ok_or(StoreError::NotFound)?,
    )
}
fn revision(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::InvalidInput("revision"))
}
fn safe_json(value: &Connection) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|_| StoreError::CorruptState)
}
pub(super) async fn serialize_key(tx: &Transaction<'_>, key: &str) -> Result<(), StoreError> {
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 30))",
        &[&key],
    )
    .await?;
    Ok(())
}
fn page_limit(limit: u32) -> Result<i64, StoreError> {
    if limit > 100 {
        return Err(StoreError::InvalidInput("limit"));
    }
    Ok(i64::from(if limit == 0 { 50 } else { limit }))
}
fn validate_candidate_input(input: &SaveGithub<'_>) -> Result<String, StoreError> {
    validate_key(input.key)?;
    validate_name(input.name, "display_name")?;
    if input.name.len() > 120
        || input.app_id.is_empty()
        || input.app_id.len() > 20
        || !input.app_id.bytes().all(|v| v.is_ascii_digit())
        || input.app_id.parse::<u64>().ok().is_none_or(|v| v == 0)
        || input.private_key_pem.is_empty()
        || input.private_key_pem.len() > 32768
    {
        return Err(StoreError::InvalidInput("github credentials"));
    }
    if !input.id.is_empty() {
        validate_id(input.id, "connection_id")?;
    }
    let safe = serde_json::to_string(&(input.id, input.expected, input.name, input.app_id))
        .map_err(|_| StoreError::CorruptState)?;
    Ok(safe)
}
// Supply a key only while its accepted operation matches the current candidate.
async fn candidate_credential(client: &Client, id: &str) -> Result<Option<String>, StoreError> {
    client
            .query_opt(
                "SELECT k.value FROM workflow_connection_operations o
                 JOIN workflow_connections c ON c.id=o.connection_id
                 LEFT JOIN workflow_connection_credentials k ON k.connection_id=c.id AND k.revision_id=o.credential_revision
                 WHERE o.id=$1 AND o.state='accepted' AND c.enabled AND c.revision=o.connection_revision
                   AND c.candidate_credential_revision=o.credential_revision",
                &[&id],
            )
            .await?
            .map(|row| row.get::<_, Option<String>>(0).ok_or(StoreError::CorruptState))
            .transpose()
}
fn model_json(models: &[String]) -> Result<String, StoreError> {
    if models.len() > 1000
        || models.iter().any(|m| {
            m.len() > 200
                || m.is_empty()
                || !m
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
        })
    {
        return Err(StoreError::InvalidInput("models"));
    }
    serde_json::to_string(models).map_err(|_| StoreError::CorruptState)
}
impl WorkflowStore {
    /// Save an immutable candidate, comparing credential retries only in protected storage.
    /// # Errors
    /// Reject invalid input, stale writes, conflicting retries or database failure.
    pub async fn save_github_connection(
        &self,
        input: SaveGithub<'_>,
    ) -> Result<Operation, StoreError> {
        let safe = validate_candidate_input(&input)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        serialize_key(&tx, input.key).await?;
        if let Some(row) = tx
            .query_opt(
                "SELECT * FROM workflow_connection_operations WHERE idempotency_key=$1",
                &[&input.key],
            )
            .await?
        {
            if row.get::<_, String>("kind") != "save"
                || row.get::<_, String>("safe_request") != safe
            {
                return Err(StoreError::IdempotencyConflict);
            }
            let stored: String = tx
                .query_one(
                    "SELECT value FROM workflow_connection_credentials WHERE revision_id=$1",
                    &[&row.get::<_, String>("credential_revision")],
                )
                .await?
                .get(0);
            if stored != input.private_key_pem {
                return Err(StoreError::IdempotencyConflict);
            }
            let result = operation(&row)?;
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let id = if input.id.is_empty() {
            new_id()
        } else {
            input.id.to_owned()
        };
        let credential = new_id();
        let op_id = new_id();
        if input.id.is_empty() {
            if input.expected != 0 {
                return Err(StoreError::ConfigurationRevisionConflict);
            }
            tx.execute("INSERT INTO workflow_connections (id,display_name,provider,capability,revision,enabled,app_id,candidate_credential_revision,status) VALUES ($1,$2,'github','source_control',1,true,$3,$4,'pending')", &[&id,&input.name,&input.app_id,&credential]).await?;
        } else {
            let current = locked(&tx, &id).await?;
            if current.status == "deleted" {
                return Err(StoreError::NotFound);
            }
            if current.app_id != input.app_id {
                return Err(StoreError::InvalidInput("App identity is immutable"));
            }
            if current.revision != input.expected {
                return Err(StoreError::ConfigurationRevisionConflict);
            }
            tx.execute("UPDATE workflow_connections SET revision=revision+1,display_name=$2,app_id=$3,candidate_credential_revision=$4,safe_reason='',status=CASE WHEN enabled THEN 'pending' ELSE 'disabled' END WHERE id=$1", &[&id,&input.name,&input.app_id,&credential]).await?;
        }
        tx.execute("INSERT INTO workflow_connection_credentials (revision_id,connection_id,value) VALUES ($1,$2,$3)", &[&credential,&id,&input.private_key_pem]).await?;
        let value = locked(&tx, &id).await?;
        tx.execute("INSERT INTO workflow_connection_operations (id,idempotency_key,connection_id,kind,safe_request,credential_revision,connection_revision,state,result) VALUES ($1,$2,$3,'save',$4,$5,$6,'accepted',$7)", &[&op_id,&input.key,&id,&safe,&credential,&revision(value.revision)?,&safe_json(&value)?]).await?;
        let result = operation(
            &tx.query_one(
                "SELECT * FROM workflow_connection_operations WHERE id=$1",
                &[&op_id],
            )
            .await?,
        )?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Change enabled state under an exact revision and retained retry identity.
    /// # Errors
    /// Reject stale or conflicting commands.
    pub async fn set_connection_enabled(
        &self,
        key: &str,
        id: &str,
        expected: u64,
        enabled: bool,
    ) -> Result<Operation, StoreError> {
        self.change_connection_state(key, id, expected, StateChange::SetEnabled(enabled))
            .await
    }
    /// Permanently remove a connection from active configuration while retaining frozen references.
    /// # Errors
    /// Reject missing connections, stale revisions and conflicting commands.
    pub async fn delete_connection(
        &self,
        key: &str,
        id: &str,
        expected: u64,
    ) -> Result<Operation, StoreError> {
        self.change_connection_state(key, id, expected, StateChange::Delete)
            .await
    }
    async fn change_connection_state(
        &self,
        key: &str,
        id: &str,
        expected: u64,
        change: StateChange,
    ) -> Result<Operation, StoreError> {
        validate_key(key)?;
        validate_id(id, "connection_id")?;
        let (kind, safe) = match change {
            StateChange::SetEnabled(enabled) => {
                ("enable", serde_json::to_string(&(id, expected, enabled)))
            }
            StateChange::Delete => ("delete", serde_json::to_string(&(id, expected))),
        };
        let safe = safe.map_err(|_| StoreError::CorruptState)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        serialize_key(&tx, key).await?;
        if let Some(row) = tx
            .query_opt(
                "SELECT * FROM workflow_connection_operations WHERE idempotency_key=$1",
                &[&key],
            )
            .await?
        {
            if row.get::<_, String>("kind") != kind || row.get::<_, String>("safe_request") != safe
            {
                return Err(StoreError::IdempotencyConflict);
            }
            let result = operation(&row)?;
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let current = locked(&tx, id).await?;
        if current.status == "deleted" {
            return Err(StoreError::NotFound);
        }
        if current.revision != expected {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        match change {
            StateChange::SetEnabled(enabled) => {
                // Re-enabling preserves the active revision and never promotes a candidate.
                tx.execute("UPDATE workflow_connections SET enabled=$2,revision=revision+1,status=CASE WHEN NOT $2 THEN 'disabled' WHEN active_credential_revision<>'' THEN 'ready' ELSE 'failed' END,safe_reason='' WHERE id=$1", &[&id,&enabled]).await?;
            }
            StateChange::Delete => {
                tx.execute("UPDATE workflow_connections SET enabled=false,revision=revision+1,status='deleted',candidate_credential_revision='',safe_reason='' WHERE id=$1", &[&id]).await?;
            }
        }
        let value = locked(&tx, id).await?;
        let op = new_id();
        tx.execute("INSERT INTO workflow_connection_operations (id,idempotency_key,connection_id,kind,safe_request,connection_revision,state,result) VALUES ($1,$2,$3,$4,$5,$6,'completed',$7)", &[&op,&key,&id,&kind,&safe,&revision(value.revision)?,&safe_json(&value)?]).await?;
        let result = operation(
            &tx.query_one(
                "SELECT * FROM workflow_connection_operations WHERE id=$1",
                &[&op],
            )
            .await?,
        )?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Read a safe receipt after an ambiguous credential submission.
    /// # Errors
    /// Reject unknown keys or database failure.
    pub async fn get_connection_operation(&self, key: &str) -> Result<Operation, StoreError> {
        validate_key(key)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let result = operation(
            &client
                .query_opt(
                    "SELECT * FROM workflow_connection_operations WHERE idempotency_key=$1",
                    &[&key],
                )
                .await?
                .ok_or(StoreError::NotFound)?,
        )?;
        driver.abort();
        Ok(result)
    }
    /// Load a candidate only for the trusted validation adapter; never expose via reads.
    /// # Errors
    /// Reject unknown operations or database failure.
    pub async fn github_candidate(&self, id: &str) -> Result<GithubCandidate, StoreError> {
        validate_id(id, "operation_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client
            .query_opt(
                "SELECT * FROM workflow_connection_operations WHERE id=$1 AND kind='save'",
                &[&id],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let op = operation(&row)?;
        if op.connection.capability != "source_control" {
            return Err(StoreError::InvalidInput("capability"));
        }
        let value = candidate_credential(&client, id).await?;
        driver.abort();
        Ok(GithubCandidate {
            app_id: op.connection.app_id.clone(),
            operation: op,
            private_key_pem: value,
        })
    }
    /// Read current connection settings and the exact candidate operation eligible for validation.
    /// # Errors
    /// Reject invalid IDs, missing connections or database failure.
    pub async fn connection_settings(&self, id: &str) -> Result<(Connection, String), StoreError> {
        validate_id(id, "connection_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client.query_opt(
            "SELECT c.*, COALESCE((SELECT o.id FROM workflow_connection_operations o
                WHERE o.connection_id=c.id AND o.kind='save' AND o.state='accepted'
                  AND o.connection_revision=c.revision AND o.credential_revision=c.candidate_credential_revision
                  AND c.enabled), '') AS pending_operation_id
             FROM workflow_connections c WHERE c.id=$1 AND c.status<>'deleted'",
            &[&id],
        ).await?.ok_or(StoreError::NotFound)?;
        let result = (connection(&row)?, row.get("pending_operation_id"));
        driver.abort();
        Ok(result)
    }
    /// Read the active credential for a bounded validation of the exact connection revision.
    /// # Errors
    /// Reject disabled, unvalidated, missing or stale connections.
    pub async fn active_connection_credential(
        &self,
        id: &str,
        expected: u64,
    ) -> Result<(Connection, String), StoreError> {
        validate_id(id, "connection_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client.query_opt(
            "SELECT c.*, k.value AS active_value FROM workflow_connections c
             JOIN workflow_connection_credentials k ON k.connection_id=c.id AND k.revision_id=c.active_credential_revision
             WHERE c.id=$1 AND c.enabled AND c.revision=$2",
            &[&id, &revision(expected)?],
        ).await?.ok_or(StoreError::ConfigurationRevisionConflict)?;
        let result = (connection(&row)?, row.get("active_value"));
        driver.abort();
        Ok(result)
    }
    /// Accept a safe validation result only for the exact current candidate generation.
    /// # Errors
    /// Reject unknown operations, unsafe reasons or database failure.
    pub async fn finish_github_validation(
        &self,
        id: &str,
        reason: &str,
    ) -> Result<Operation, StoreError> {
        self.finish_provider_validation(id, reason, &[]).await
    }
    /// Complete provider validation under the candidate revision fence.
    /// # Errors
    /// Reject unsafe outcomes, stale operations or unavailable storage.
    pub async fn finish_provider_validation(
        &self,
        id: &str,
        reason: &str,
        models: &[String],
    ) -> Result<Operation, StoreError> {
        let models_json = model_json(models)?;
        if !matches!(
            reason,
            "" | "github_key_invalid"
                | "github_app_identity_mismatch"
                | "github_authentication_failed"
                | "github_access_denied"
                | "github_rate_limited"
                | "github_unavailable"
                | "github_response_invalid"
                | "provider_authentication_failed"
                | "provider_access_denied"
                | "provider_rate_limited"
                | "provider_unavailable"
                | "provider_response_invalid"
        ) {
            return Err(StoreError::InvalidInput("validation reason"));
        }
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        // Lock connection first, like all writers; operation completion then fences concurrent validations.
        let initial = tx
            .query_opt(
                "SELECT connection_id FROM workflow_connection_operations WHERE id=$1",
                &[&id],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let current = locked(&tx, initial.get("connection_id")).await?;
        let row = tx
            .query_one(
                "SELECT * FROM workflow_connection_operations WHERE id=$1 FOR UPDATE",
                &[&id],
            )
            .await?;
        if row.get::<_, String>("state") != "accepted" {
            let result = operation(&row)?;
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let candidate: String = row.get("credential_revision");
        let state = if !current.enabled
            || revision(current.revision)? != row.get::<_, i64>("connection_revision")
            || current.candidate_credential_revision != candidate
        {
            "superseded"
        } else {
            if matches!(
                reason,
                "github_unavailable"
                    | "github_rate_limited"
                    | "provider_unavailable"
                    | "provider_rate_limited"
            ) {
                tx.execute(
                    "UPDATE workflow_connections SET safe_reason=$2 WHERE id=$1",
                    &[&current.id, &reason],
                )
                .await?;
            } else if reason.is_empty() {
                tx.execute("UPDATE workflow_connections SET active_credential_revision=$2,candidate_credential_revision='',status='ready',safe_reason='',models=$3 WHERE id=$1", &[&current.id,&candidate,&models_json]).await?;
            } else {
                tx.execute("UPDATE workflow_connections SET candidate_credential_revision='',status=CASE WHEN active_credential_revision='' THEN 'failed' ELSE 'ready' END,safe_reason=$2 WHERE id=$1", &[&current.id,&reason]).await?;
            }
            if matches!(
                reason,
                "github_unavailable"
                    | "github_rate_limited"
                    | "provider_unavailable"
                    | "provider_rate_limited"
            ) {
                "accepted"
            } else if reason.is_empty() {
                "completed"
            } else {
                "failed"
            }
        };
        let value = locked(&tx, &current.id).await?;
        tx.execute("UPDATE workflow_connection_operations SET state=$2,safe_reason=$3,result=$4 WHERE id=$1", &[&id,&state,&reason,&safe_json(&value)?]).await?;
        let result = operation(
            &tx.query_one(
                "SELECT * FROM workflow_connection_operations WHERE id=$1",
                &[&id],
            )
            .await?,
        )?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// List bounded safe connection projections.
    /// # Errors
    /// Reject malformed pagination or database failure.
    pub async fn list_connections(
        &self,
        after: &str,
        limit: u32,
    ) -> Result<Vec<Connection>, StoreError> {
        if !after.is_empty() {
            validate_id(after, "after_id")?;
        }
        let limit = page_limit(limit)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let result = client
            .query(
                "SELECT * FROM workflow_connections WHERE id>$1 AND status<>'deleted' ORDER BY id LIMIT $2",
                &[&after, &limit],
            )
            .await?
            .iter()
            .map(connection)
            .collect();
        driver.abort();
        result
    }
    /// List bounded immutable operation outcomes for a connection.
    /// # Errors
    /// Reject malformed pagination or database failure.
    pub async fn list_connection_operations(
        &self,
        id: &str,
        after: &str,
        limit: u32,
    ) -> Result<Vec<Operation>, StoreError> {
        validate_id(id, "connection_id")?;
        if !after.is_empty() {
            validate_id(after, "after_id")?;
        }
        let limit = page_limit(limit)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let result=client.query("SELECT * FROM workflow_connection_operations WHERE connection_id=$1 AND id>$2 ORDER BY id LIMIT $3", &[&id,&after,&limit]).await?.iter().map(operation).collect();
        driver.abort();
        result
    }
}
