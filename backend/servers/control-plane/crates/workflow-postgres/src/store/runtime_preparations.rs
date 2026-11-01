//! Durable, stage-fenced runtime preparation authority. New records never retain credentials.
use super::{StoreError, WorkflowStore, connections, new_id, validate_id, validate_key};
pub use ayni_orchestrator_events::runtime_preparation::{
    Binding, Evidence, FreezeInputs, FrozenFile, FrozenInputs, Operation, RuntimeProfile,
    StageAttempt, StageClaim, StageReport,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use tokio_postgres::{Row, Transaction};
mod admission;
mod execution;
#[cfg(test)]
mod tests;
mod validation;

#[derive(Clone, Debug, Serialize)]
pub struct Start<'a> {
    pub key: &'a str,
    pub repository_id: &'a str,
    pub inspection_id: &'a str,
    pub registry_connection_id: &'a str,
    pub signer_id: &'a str,
    pub platform: &'a str,
    pub max_seconds: u32,
    pub max_builds: u32,
    pub allow_build: bool,
    pub allow_replace: bool,
}
/// Capability-bearing results deliberately have no Debug/Serialize implementations.
pub enum Credential {
    Source {
        credential: super::repositories::SourceCredential,
        installation_id: u64,
        repository_id: u64,
    },
    Registry {
        username: String,
        password: String,
    },
    None,
}
fn json<T: Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|_| StoreError::CorruptState)
}
fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes.as_ref()))
}
fn unsigned(row: &Row, key: &str) -> Result<u64, StoreError> {
    u64::try_from(row.get::<_, i64>(key)).map_err(|_| StoreError::CorruptState)
}
fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::InvalidInput("generation"))
}
fn operation(row: &Row) -> Result<Operation, StoreError> {
    Ok(Operation {
        id: row.get("id"),
        idempotency_key: row.get("idempotency_key"),
        repository_id: row.get("repository_id"),
        inspection_id: row.get("inspection_id"),
        registry_connection_id: row.get("registry_connection_id"),
        signer_id: row.get("signer_id"),
        platform: row.get("platform"),
        state: row.get("state"),
        stage: row.get("stage"),
        safe_reason: row.get("safe_reason"),
        generation: unsigned(row, "generation")?,
        allow_build: row.get("allow_build"),
        allow_replace: row.get("allow_replace"),
        max_builds: u32::try_from(row.get::<_, i32>("max_builds"))
            .map_err(|_| StoreError::CorruptState)?,
        builds_started: u32::try_from(row.get::<_, i32>("builds_started"))
            .map_err(|_| StoreError::CorruptState)?,
        deadline_unix_ms: unsigned(row, "deadline_unix_ms")?,
        accepted_at_unix_ms: unsigned(row, "accepted_at_unix_ms")?,
        updated_at_unix_ms: unsigned(row, "updated_at_unix_ms")?,
        frozen_input_digest: row.get("frozen_input_digest"),
        frozen: serde_json::from_str(row.get("frozen")).map_err(|_| StoreError::CorruptState)?,
        evidence: serde_json::from_str(row.get("evidence"))
            .map_err(|_| StoreError::CorruptState)?,
    })
}
fn attempt(row: &Row) -> Result<StageAttempt, StoreError> {
    Ok(StageAttempt {
        id: row.get("id"),
        operation_id: row.get("operation_id"),
        stage: row.get("stage"),
        generation: unsigned(row, "generation")?,
        command_digest: row.get("command_digest"),
        pod_uid: row.get("pod_uid"),
        state: row.get("state"),
        deadline_unix_ms: unsigned(row, "deadline_unix_ms")?,
        accepted_at_unix_ms: unsigned(row, "accepted_at_unix_ms")?,
        report_digest: row.get("report_digest"),
    })
}
async fn now(tx: &Transaction<'_>) -> Result<u64, StoreError> {
    let value: i64 = tx
        .query_one(
            "SELECT (extract(epoch FROM clock_timestamp())*1000)::bigint AS now",
            &[],
        )
        .await?
        .get("now");
    u64::try_from(value).map_err(|_| StoreError::CorruptState)
}
async fn locked(tx: &Transaction<'_>, id: &str) -> Result<Operation, StoreError> {
    let row = tx
        .query_opt(
            "SELECT * FROM workflow_runtime_preparations WHERE id=$1 FOR UPDATE",
            &[&id],
        )
        .await?
        .ok_or(StoreError::NotFound)?;
    operation(&row)
}
async fn receipt(
    tx: &Transaction<'_>,
    key: &str,
    kind: &str,
    expected: &str,
) -> Result<Option<Operation>, StoreError> {
    let Some(row)=tx.query_opt("SELECT kind,request_digest,result FROM workflow_runtime_preparation_receipts WHERE idempotency_key=$1",&[&key]).await? else{return Ok(None)};
    if row.get::<_, String>("kind") != kind || row.get::<_, String>("request_digest") != expected {
        return Err(StoreError::IdempotencyConflict);
    }
    serde_json::from_str(row.get("result"))
        .map(Some)
        .map_err(|_| StoreError::CorruptState)
}
async fn save_receipt(
    tx: &Transaction<'_>,
    key: &str,
    kind: &str,
    request_digest: &str,
    op: &Operation,
) -> Result<(), StoreError> {
    tx.execute("INSERT INTO workflow_runtime_preparation_receipts(idempotency_key,command_id,operation_id,kind,request_digest,result) VALUES($1,$2,$3,$4,$5,$6)",&[&key,&new_id(),&op.id,&kind,&request_digest,&json(op)?]).await?;
    Ok(())
}
impl WorkflowStore {
    /// Read retained operation evidence; this is not worker authorization.
    /// # Errors
    /// Reject absent/invalid identities or unavailable retained state.
    pub async fn get_runtime_preparation(&self, id: &str) -> Result<Operation, StoreError> {
        validate_id(id, "operation_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let result = operation(
            &client
                .query_opt(
                    "SELECT * FROM workflow_runtime_preparations WHERE id=$1",
                    &[&id],
                )
                .await?
                .ok_or(StoreError::NotFound)?,
        )?;
        driver.abort();
        Ok(result)
    }
    /// Recover the immutable acceptance receipt after a lost command response.
    /// # Errors
    /// Reject invalid/absent command keys and unavailable retained state.
    pub async fn get_runtime_preparation_receipt(
        &self,
        key: &str,
    ) -> Result<Operation, StoreError> {
        validate_key(key)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client
            .query_opt(
                "SELECT result FROM workflow_runtime_preparation_receipts WHERE idempotency_key=$1",
                &[&key],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let result =
            serde_json::from_str(row.get("result")).map_err(|_| StoreError::CorruptState)?;
        driver.abort();
        Ok(result)
    }
    /// Page operation history for one catalog repository or all repositories.
    /// # Errors
    /// Reject invalid pagination and unavailable retained state.
    pub async fn list_runtime_preparations(
        &self,
        repository_id: &str,
        after_id: &str,
        limit: u32,
    ) -> Result<(Vec<Operation>, String), StoreError> {
        if !repository_id.is_empty() {
            validate_id(repository_id, "repository_id")?;
        }
        if !after_id.is_empty() {
            validate_id(after_id, "after_id")?;
        }
        if limit > 100 {
            return Err(StoreError::InvalidInput("limit"));
        }
        let limit = i64::from(if limit == 0 { 50 } else { limit });
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows=client.query("SELECT * FROM workflow_runtime_preparations WHERE ($1='' OR repository_id=$1) AND ($2='' OR id<$2) ORDER BY id DESC LIMIT $3",&[&repository_id,&after_id,&(limit+1)]).await?;
        let mut values = rows.iter().map(operation).collect::<Result<Vec<_>, _>>()?;
        let page_size = usize::try_from(limit).map_err(|_| StoreError::CorruptState)?;
        let next = if values.len() > page_size {
            values.truncate(page_size);
            values.last().map(|v| v.id.clone()).unwrap_or_default()
        } else {
            String::new()
        };
        driver.abort();
        Ok((values, next))
    }
    /// Enumerate retained operations requiring supervisor reconciliation.
    /// # Errors
    /// Reject unavailable state or an excessive requested bound.
    pub async fn pending_runtime_preparations(
        &self,
        limit: u32,
    ) -> Result<Vec<Operation>, StoreError> {
        if !(1..=100).contains(&limit) {
            return Err(StoreError::InvalidInput("limit"));
        }
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows=client.query("SELECT * FROM workflow_runtime_preparations WHERE state IN ('pending','running','stopping','cancelling','cleanup_pending') ORDER BY id LIMIT $1",&[&i64::from(limit)]).await?;
        let values = rows.iter().map(operation).collect::<Result<Vec<_>, _>>()?;
        driver.abort();
        Ok(values)
    }
    /// Read append-only stage history for a retained operation.
    /// # Errors
    /// Reject absent operations or unavailable evidence.
    pub async fn runtime_preparation_attempts(
        &self,
        id: &str,
    ) -> Result<Vec<StageAttempt>, StoreError> {
        validate_id(id, "operation_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows=client.query("SELECT * FROM workflow_runtime_preparation_attempts WHERE operation_id=$1 ORDER BY generation",&[&id]).await?;
        let values = rows.iter().map(attempt).collect::<Result<Vec<_>, _>>()?;
        driver.abort();
        Ok(values)
    }
}
