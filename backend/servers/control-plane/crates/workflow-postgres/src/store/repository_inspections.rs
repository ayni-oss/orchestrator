//! Frozen committed-source inspection receipts; these never assert runtime readiness.
use super::repositories::Repository;
use super::{StoreError, WorkflowStore, connections, new_id, validate_id, validate_key};
use serde::{Deserialize, Serialize};
use tokio_postgres::Row;

#[derive(Clone, Serialize)]
pub struct Inspect<'a> {
    pub key: &'a str,
    pub repository_id: &'a str,
    pub expected_connection_revision: u64,
    pub commit_sha: &'a str,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Finding {
    pub code: String,
    pub path: String,
    pub action: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InspectionResult {
    pub policy_digest: String,
    pub lock_digest: String,
    pub lock_fingerprint: String,
    pub ayni_version: String,
    pub lock_schema: String,
    pub tools: Vec<String>,
    pub trust_key_ids: Vec<String>,
    pub findings: Vec<Finding>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Inspection {
    pub id: String,
    pub idempotency_key: String,
    pub repository_id: String,
    pub connection_revision: u64,
    pub commit_sha: String,
    pub state: String,
    pub safe_reason: String,
    pub result: Option<InspectionResult>,
    pub accepted_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}
fn unsigned(row: &Row, name: &str) -> Result<u64, StoreError> {
    u64::try_from(row.get::<_, i64>(name)).map_err(|_| StoreError::CorruptState)
}
fn inspection(row: &Row) -> Result<Inspection, StoreError> {
    let result: String = row.get("result");
    Ok(Inspection {
        id: row.get("id"),
        idempotency_key: row.get("idempotency_key"),
        repository_id: row.get("repository_id"),
        connection_revision: unsigned(row, "connection_revision")?,
        commit_sha: row.get("commit_sha"),
        state: row.get("state"),
        safe_reason: row.get("safe_reason"),
        result: if result.is_empty() {
            None
        } else {
            Some(serde_json::from_str(&result).map_err(|_| StoreError::CorruptState)?)
        },
        accepted_at_unix_ms: unsigned(row, "accepted_at_unix_ms")?,
        updated_at_unix_ms: unsigned(row, "updated_at_unix_ms")?,
    })
}
fn validate_commit(sha: &str) -> Result<(), StoreError> {
    if sha.len() != 40
        || !sha
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
    {
        return Err(StoreError::InvalidInput("commit_sha"));
    }
    Ok(())
}
fn validate_observation(value: &Result<InspectionResult, &str>) -> Result<(), StoreError> {
    if let Err(reason) = value {
        if !matches!(
            *reason,
            "github_key_invalid"
                | "github_authentication_failed"
                | "github_access_denied"
                | "github_rate_limited"
                | "github_unavailable"
                | "github_response_invalid"
                | "github_repository_unavailable"
                | "github_source_changed"
                | "github_commit_unavailable"
                | "source_bounds_exceeded"
                | "source_path_unsupported"
                | "inspection_unavailable"
        ) {
            return Err(StoreError::InvalidInput("inspection reason"));
        }
    } else if serde_json::to_vec(value.as_ref().map_err(|_| StoreError::CorruptState)?)
        .map_err(|_| StoreError::CorruptState)?
        .len()
        > 65_536
    {
        return Err(StoreError::InvalidInput("inspection result"));
    }
    Ok(())
}
impl WorkflowStore {
    /// Read the safe catalog identity used to obtain exact committed source.
    /// # Errors
    /// Reject invalid IDs, absent repositories and database errors.
    pub async fn inspection_repository(&self, id: &str) -> Result<Repository, StoreError> {
        validate_id(id, "repository_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client
            .query_opt("SELECT * FROM workflow_repositories WHERE id=$1", &[&id])
            .await?
            .ok_or(StoreError::NotFound)?;
        let result = Repository {
            id: row.get("id"),
            connection_id: row.get("connection_id"),
            installation_id: unsigned(&row, "installation_id")?,
            provider_repository_id: unsigned(&row, "provider_repository_id")?,
            full_name: row.get("full_name"),
            default_branch: row.get("default_branch"),
            private: row.get("private"),
            connected_at_unix_ms: unsigned(&row, "connected_at_unix_ms")?,
        };
        driver.abort();
        Ok(result)
    }
    /// Accept exact committed-source intent before external access; recover exact retries.
    /// # Errors
    /// Reject invalid input, reused keys, unavailable source generations and database errors.
    pub async fn begin_repository_inspection(
        &self,
        input: Inspect<'_>,
    ) -> Result<Inspection, StoreError> {
        validate_key(input.key)?;
        validate_id(input.repository_id, "repository_id")?;
        validate_commit(input.commit_sha)?;
        let expected = i64::try_from(input.expected_connection_revision)
            .map_err(|_| StoreError::InvalidInput("connection_revision"))?;
        if expected <= 0 {
            return Err(StoreError::InvalidInput("connection_revision"));
        }
        let safe = serde_json::to_string(&input).map_err(|_| StoreError::CorruptState)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, input.key).await?;
        if let Some(row) = tx
            .query_opt(
                "SELECT * FROM workflow_repository_inspections WHERE idempotency_key=$1",
                &[&input.key],
            )
            .await?
        {
            if row.get::<_, String>("safe_request") != safe {
                return Err(StoreError::IdempotencyConflict);
            }
            let result = inspection(&row)?;
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let row = tx
            .query_opt(
                "SELECT connection_id FROM workflow_repositories WHERE id=$1",
                &[&input.repository_id],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let connection: String = row.get(0);
        let c = connections::locked(&tx, &connection).await?;
        if !c.enabled
            || c.provider != "github"
            || c.capability != "source_control"
            || c.revision != input.expected_connection_revision
            || c.active_credential_revision.is_empty()
        {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        let row=tx.query_one("INSERT INTO workflow_repository_inspections (id,idempotency_key,safe_request,repository_id,connection_id,connection_revision,credential_revision,commit_sha,state) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'accepted') RETURNING *",&[&new_id(),&input.key,&safe,&input.repository_id,&connection,&expected,&c.active_credential_revision,&input.commit_sha]).await?;
        let result = inspection(&row)?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Fence late observations against the accepted connection and credential revision.
    /// # Errors
    /// Reject malformed safe reports, absent keys and database errors.
    pub async fn finish_repository_inspection(
        &self,
        key: &str,
        observed: Result<InspectionResult, &str>,
    ) -> Result<Inspection, StoreError> {
        validate_key(key)?;
        validate_observation(&observed)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let original=tx.query_opt("SELECT connection_id FROM workflow_repository_inspections WHERE idempotency_key=$1",&[&key]).await?.ok_or(StoreError::NotFound)?;
        let c = connections::locked(&tx, original.get("connection_id")).await?;
        let row = tx
            .query_one(
                "SELECT * FROM workflow_repository_inspections WHERE idempotency_key=$1 FOR UPDATE",
                &[&key],
            )
            .await?;
        let op = inspection(&row)?;
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
                Ok(value) => {
                    result = serde_json::to_string(&value).map_err(|_| StoreError::CorruptState)?;
                    ("completed", "")
                }
                Err(reason) => (
                    if matches!(
                        reason,
                        "github_unavailable" | "github_rate_limited" | "inspection_unavailable"
                    ) {
                        "accepted"
                    } else {
                        "failed"
                    },
                    reason,
                ),
            }
        };
        let row=tx.query_one("UPDATE workflow_repository_inspections SET state=$2,safe_reason=$3,result=$4,updated_at_unix_ms=(extract(epoch FROM clock_timestamp())*1000)::bigint WHERE idempotency_key=$1 RETURNING *",&[&key,&state,&reason,&result]).await?;
        let result = inspection(&row)?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Read a safe retained receipt without resuming provider access.
    /// # Errors
    /// Reject malformed keys, absent inspections and database errors.
    pub async fn get_repository_inspection(&self, key: &str) -> Result<Inspection, StoreError> {
        validate_key(key)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client
            .query_opt(
                "SELECT * FROM workflow_repository_inspections WHERE idempotency_key=$1",
                &[&key],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let result = inspection(&row)?;
        driver.abort();
        Ok(result)
    }
    /// List bounded historical inspections for one catalog repository.
    /// # Errors
    /// Reject invalid bounds, absent repositories and database errors.
    pub async fn list_repository_inspections(
        &self,
        repository_id: &str,
        after: &str,
        limit: u32,
    ) -> Result<(Vec<Inspection>, String), StoreError> {
        validate_id(repository_id, "repository_id")?;
        if !after.is_empty() {
            validate_id(after, "after_id")?;
        }
        if limit > 100 {
            return Err(StoreError::InvalidInput("limit"));
        }
        let limit = i64::from(if limit == 0 { 50 } else { limit });
        let (client, driver) = crate::connect(&self.database_url).await?;
        client
            .query_opt(
                "SELECT id FROM workflow_repositories WHERE id=$1",
                &[&repository_id],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let rows=client.query("SELECT * FROM workflow_repository_inspections WHERE repository_id=$1 AND ($2='' OR id<$2) ORDER BY id DESC LIMIT $3",&[&repository_id,&after,&(limit+1)]).await?;
        let mut values = rows.iter().map(inspection).collect::<Result<Vec<_>, _>>()?;
        let next = if values.len() > usize::try_from(limit).unwrap_or(0) {
            values.pop();
            values.last().map_or_else(String::new, |v| v.id.clone())
        } else {
            String::new()
        };
        driver.abort();
        Ok((values, next))
    }
}
#[cfg(test)]
mod tests {
    use super::validate_commit;
    #[test]
    fn only_exact_lowercase_commit_ids_are_accepted() {
        assert!(validate_commit(&"a".repeat(40)).is_ok());
        for value in [
            "main".to_owned(),
            "a".repeat(39),
            "A".repeat(40),
            "g".repeat(40),
        ] {
            assert!(validate_commit(&value).is_err());
        }
    }
}
