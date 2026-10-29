//! Public signer metadata referencing admin-owned immutable Kubernetes objects.
//! This module does not read or retain signing material.
use super::{
    StoreError, WorkflowStore, connections, new_id, validate_id, validate_key, validate_name,
};
use serde::{Deserialize, Serialize};
use tokio_postgres::{Row, Transaction};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Signer {
    pub id: String,
    pub display_name: String,
    pub key_id: String,
    pub public_key: String,
    pub secret_name: String,
    pub secret_uid: String,
    pub active_revision: String,
    pub revision: u64,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct SaveSigner<'a> {
    pub key: &'a str,
    pub id: &'a str,
    pub expected_revision: u64,
    pub display_name: &'a str,
    pub key_id: &'a str,
    pub public_key: &'a str,
    pub secret_name: &'a str,
    pub secret_uid: &'a str,
}
fn signer(row: &Row) -> Result<Signer, StoreError> {
    Ok(Signer {
        id: row.get("id"),
        display_name: row.get("display_name"),
        key_id: row.get("key_id"),
        public_key: row.get("public_key"),
        secret_name: row.get("secret_name"),
        secret_uid: row.get("secret_uid"),
        active_revision: row.get("active_revision"),
        revision: u64::try_from(row.get::<_, i64>("revision"))
            .map_err(|_| StoreError::CorruptState)?,
        enabled: row.get("enabled"),
    })
}
fn validate(input: &SaveSigner<'_>) -> Result<String, StoreError> {
    validate_key(input.key)?;
    if !input.id.is_empty() {
        validate_id(input.id, "signer_id")?;
    }
    validate_name(input.display_name, "display_name")?;
    if input.display_name.len() > 120
        || input.key_id.is_empty()
        || input.key_id.len() > 128
        || !input
            .key_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(StoreError::InvalidInput("signer metadata"));
    }
    if input.public_key.len() != 64
        || !input
            .public_key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(StoreError::InvalidInput("signer public_key"));
    }
    let name = input
        .secret_name
        .strip_prefix("ayni-runtime-signer-")
        .ok_or(StoreError::InvalidInput("signer secret_name"))?;
    if input.secret_name.len() > 63
        || name.is_empty()
        || !name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        || !name.ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(StoreError::InvalidInput("signer secret_name"));
    }
    let uid = uuid::Uuid::parse_str(input.secret_uid)
        .map_err(|_| StoreError::InvalidInput("signer secret_uid"))?;
    if uid.to_string() != input.secret_uid {
        return Err(StoreError::InvalidInput("signer secret_uid"));
    }
    i64::try_from(input.expected_revision)
        .map_err(|_| StoreError::InvalidInput("signer_revision"))?;
    serde_json::to_string(input).map_err(|_| StoreError::CorruptState)
}
async fn locked(tx: &Transaction<'_>, id: &str) -> Result<Signer, StoreError> {
    signer(
        &tx.query_opt(
            "SELECT * FROM workflow_runtime_signers WHERE id=$1 FOR UPDATE",
            &[&id],
        )
        .await?
        .ok_or(StoreError::NotFound)?,
    )
}
async fn replay(
    tx: &Transaction<'_>,
    key: &str,
    kind: &str,
    safe: &str,
) -> Result<Option<Signer>, StoreError> {
    let Some(row) = tx
        .query_opt(
            "SELECT * FROM workflow_runtime_signer_receipts WHERE idempotency_key=$1",
            &[&key],
        )
        .await?
    else {
        return Ok(None);
    };
    if row.get::<_, String>("kind") != kind || row.get::<_, String>("safe_request") != safe {
        return Err(StoreError::IdempotencyConflict);
    }
    Ok(Some(
        serde_json::from_str(row.get::<_, &str>("result")).map_err(|_| StoreError::CorruptState)?,
    ))
}
async fn retain(
    tx: &Transaction<'_>,
    key: &str,
    kind: &str,
    safe: &str,
    value: &Signer,
) -> Result<(), StoreError> {
    let result = serde_json::to_string(value).map_err(|_| StoreError::CorruptState)?;
    tx.execute("INSERT INTO workflow_runtime_signer_revisions(revision_id,signer_id,configuration) VALUES($1,$2,$3)",&[&value.active_revision,&value.id,&result]).await?;
    tx.execute("INSERT INTO workflow_runtime_signer_receipts(idempotency_key,command_id,signer_id,kind,safe_request,result) VALUES($1,$2,$3,$4,$5,$6)",&[&key,&new_id(),&value.id,&kind,&safe,&result]).await?;
    Ok(())
}
impl WorkflowStore {
    /// Save or rotate public signer metadata without accessing the referenced object.
    /// # Errors
    /// Reject malformed inputs, stale revisions, key ID changes, conflicting retries and database errors.
    pub async fn save_runtime_signer(&self, input: SaveSigner<'_>) -> Result<Signer, StoreError> {
        let safe = validate(&input)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, input.key).await?;
        if let Some(value) = replay(&tx, input.key, "save", &safe).await? {
            tx.commit().await?;
            driver.abort();
            return Ok(value);
        }
        let revision = new_id();
        let id = if input.id.is_empty() {
            if input.expected_revision != 0 {
                return Err(StoreError::ConfigurationRevisionConflict);
            }
            let id = new_id();
            tx.execute("INSERT INTO workflow_runtime_signers(id,display_name,key_id,public_key,secret_name,secret_uid,active_revision,revision,enabled) VALUES($1,$2,$3,$4,$5,$6,$7,1,true)",&[&id,&input.display_name,&input.key_id,&input.public_key,&input.secret_name,&input.secret_uid,&revision]).await?;
            id
        } else {
            let current = locked(&tx, input.id).await?;
            if current.key_id != input.key_id {
                return Err(StoreError::InvalidInput("signer key_id is immutable"));
            }
            if current.revision != input.expected_revision {
                return Err(StoreError::ConfigurationRevisionConflict);
            }
            tx.execute("UPDATE workflow_runtime_signers SET display_name=$2,public_key=$3,secret_name=$4,secret_uid=$5,active_revision=$6,revision=revision+1 WHERE id=$1",&[&input.id,&input.display_name,&input.public_key,&input.secret_name,&input.secret_uid,&revision]).await?;
            input.id.to_owned()
        };
        let value = locked(&tx, &id).await?;
        retain(&tx, input.key, "save", &safe, &value).await?;
        tx.commit().await?;
        driver.abort();
        Ok(value)
    }
    /// Enable or disable a signer while retaining every public configuration revision.
    /// # Errors
    /// Reject malformed identities, stale revisions, conflicting retries and database errors.
    pub async fn set_runtime_signer_enabled(
        &self,
        key: &str,
        id: &str,
        expected: u64,
        enabled: bool,
    ) -> Result<Signer, StoreError> {
        validate_key(key)?;
        validate_id(id, "signer_id")?;
        i64::try_from(expected).map_err(|_| StoreError::InvalidInput("signer_revision"))?;
        let safe = serde_json::to_string(&(id, expected, enabled))
            .map_err(|_| StoreError::CorruptState)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, key).await?;
        if let Some(value) = replay(&tx, key, "enabled", &safe).await? {
            tx.commit().await?;
            driver.abort();
            return Ok(value);
        }
        let current = locked(&tx, id).await?;
        if current.revision != expected {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        tx.execute("UPDATE workflow_runtime_signers SET enabled=$2,active_revision=$3,revision=revision+1 WHERE id=$1",&[&id,&enabled,&new_id()]).await?;
        let value = locked(&tx, id).await?;
        retain(&tx, key, "enabled", &safe, &value).await?;
        tx.commit().await?;
        driver.abort();
        Ok(value)
    }
    /// Read the original safe result after an acknowledgement is lost.
    /// # Errors
    /// Reject malformed or absent keys and database errors.
    pub async fn get_runtime_signer_receipt(&self, key: &str) -> Result<Signer, StoreError> {
        validate_key(key)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client
            .query_opt(
                "SELECT result FROM workflow_runtime_signer_receipts WHERE idempotency_key=$1",
                &[&key],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let value =
            serde_json::from_str(row.get::<_, &str>(0)).map_err(|_| StoreError::CorruptState)?;
        driver.abort();
        Ok(value)
    }
    /// Read public signer metadata without accessing the referenced Kubernetes object.
    /// # Errors
    /// Reject malformed or absent identities and database errors.
    pub async fn get_runtime_signer(&self, id: &str) -> Result<Signer, StoreError> {
        validate_id(id, "signer_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let value = signer(
            &client
                .query_opt("SELECT * FROM workflow_runtime_signers WHERE id=$1", &[&id])
                .await?
                .ok_or(StoreError::NotFound)?,
        )?;
        driver.abort();
        Ok(value)
    }
    /// List bounded public signer metadata in stable ID order.
    /// # Errors
    /// Reject malformed bounds and database errors.
    pub async fn list_runtime_signers(
        &self,
        after: &str,
        limit: u32,
    ) -> Result<(Vec<Signer>, String), StoreError> {
        if !after.is_empty() {
            validate_id(after, "after_id")?;
        }
        if limit > 100 {
            return Err(StoreError::InvalidInput("limit"));
        }
        let limit = i64::from(if limit == 0 { 50 } else { limit });
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows = client
            .query(
                "SELECT * FROM workflow_runtime_signers WHERE id>$1 ORDER BY id LIMIT $2",
                &[&after, &(limit + 1)],
            )
            .await?;
        let mut values = rows.iter().map(signer).collect::<Result<Vec<_>, _>>()?;
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
