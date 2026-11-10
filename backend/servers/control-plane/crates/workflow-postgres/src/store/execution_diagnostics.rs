//! Retained observations and a separate delivery queue for `JetStream`.
use super::{StoreError, WorkflowStore};
use ayni_orchestrator_events::execution_diagnostics::{Diagnostic, Entry, Kind, MAX_BYTES};
use sha2::{Digest as _, Sha256};
use tokio_postgres::{Row, Transaction};

const PROJECTION: &str = "cursor,(extract(epoch FROM observed_at)*1000)::bigint,diagnostic";

pub(super) async fn append(
    tx: &Transaction<'_>,
    diagnostic: &Diagnostic,
) -> Result<(), StoreError> {
    let body = serde_json::to_string(diagnostic).map_err(|_| StoreError::CorruptState)?;
    if body.len() > MAX_BYTES {
        return Err(StoreError::InvalidInput("execution diagnostic size"));
    }
    let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
    // A scoped replay cursor never skips an earlier transaction still committing.
    let scope = format!(
        "diagnostics:{}:{}",
        diagnostic.kind.name(),
        diagnostic.execution_id
    );
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
        &[&scope],
    )
    .await?;
    tx.execute(
        "INSERT INTO workflow_execution_diagnostics(kind,execution_id,digest,diagnostic) VALUES($1,$2,$3,$4) ON CONFLICT(kind,execution_id,digest) DO NOTHING",
        &[&diagnostic.kind.name(), &diagnostic.execution_id, &digest, &body],
    ).await?;
    Ok(())
}

fn entry(row: &Row) -> Result<Entry, StoreError> {
    let payload: String = row.get(2);
    Ok(Entry {
        cursor: row.get(0),
        observed_at_unix_ms: row.get(1),
        diagnostic: serde_json::from_str(&payload).map_err(|_| StoreError::CorruptState)?,
    })
}

impl WorkflowStore {
    /// Retain a supervisor observation for an existing, scoped execution.
    /// # Errors
    /// Reject invalid lifecycle metadata or a mismatched execution identity.
    pub async fn record_execution_lifecycle(
        &self,
        diagnostic: &Diagnostic,
    ) -> Result<(), StoreError> {
        if !diagnostic.valid_lifecycle() {
            return Err(StoreError::InvalidInput("execution lifecycle diagnostic"));
        }
        let generation = i64::try_from(diagnostic.generation)
            .map_err(|_| StoreError::InvalidInput("execution generation"))?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let id = &diagnostic.execution_id;
        let known = match diagnostic.kind {
            Kind::Runtime => tx.query_opt("SELECT 1 FROM workflow_runtime_preparation_attempts WHERE operation_id=$1 AND generation=$2 AND stage=$3 AND ($4='' OR pod_uid=$4)", &[id,&generation,&diagnostic.stage,&diagnostic.pod_uid]).await?,
            Kind::Task => tx.query_opt("SELECT 1 FROM workflow_task_runs WHERE attempt_id=$1 AND generation=$2 AND ($3='' OR pod_uid=$3)", &[id,&generation,&diagnostic.pod_uid]).await?,
            Kind::Session => tx.query_opt("SELECT 1 FROM workflow_sessions WHERE id=$1 AND generation=$2", &[id,&generation]).await?,
        };
        if known.is_none() {
            return Err(StoreError::NotFound);
        }
        append(&tx, diagnostic).await?;
        tx.commit().await?;
        driver.abort();
        Ok(())
    }

    /// Read a bounded page after a cursor in one execution's history.
    /// # Errors
    /// Return an error on invalid scope, database failure, or invalid retained data.
    pub async fn list_execution_diagnostics(
        &self,
        kind: Kind,
        id: &str,
        after: i64,
    ) -> Result<Vec<Entry>, StoreError> {
        super::support::validate_id(id, "execution diagnostic id")?;
        if after < 0 {
            return Err(StoreError::InvalidInput("execution diagnostic cursor"));
        }
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows = client.query(&format!("SELECT {PROJECTION} FROM workflow_execution_diagnostics WHERE kind=$1 AND execution_id=$2 AND cursor>$3 ORDER BY cursor LIMIT 50"), &[&kind.name(),&id,&after]).await?;
        driver.abort();
        rows.iter().map(entry).collect()
    }

    /// Read a bounded batch of observations awaiting broker acknowledgement.
    /// # Errors
    /// Return a database or retained-data error.
    pub async fn pending_execution_diagnostics(&self) -> Result<Vec<Entry>, StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows = client.query(&format!("SELECT {PROJECTION} FROM workflow_execution_diagnostics WHERE published_at IS NULL ORDER BY cursor LIMIT 16"), &[]).await?;
        driver.abort();
        rows.iter().map(entry).collect()
    }

    /// Record acknowledgement without changing the retained diagnostic.
    /// # Errors
    /// Return a database error if the observation could not be updated.
    pub async fn mark_execution_diagnostic_published(&self, cursor: i64) -> Result<(), StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        client.execute("UPDATE workflow_execution_diagnostics SET published_at=clock_timestamp() WHERE cursor=$1 AND published_at IS NULL", &[&cursor]).await?;
        driver.abort();
        Ok(())
    }

    /// Expire at most one bounded batch of diagnostics after 30 days.
    /// # Errors
    /// Return a database error if retention could not advance.
    pub async fn cleanup_execution_diagnostics(&self) -> Result<(), StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        client.execute("DELETE FROM workflow_execution_diagnostics WHERE cursor IN (SELECT cursor FROM workflow_execution_diagnostics WHERE observed_at < clock_timestamp() - interval '30 days' ORDER BY observed_at LIMIT 500)", &[]).await?;
        driver.abort();
        Ok(())
    }
}
