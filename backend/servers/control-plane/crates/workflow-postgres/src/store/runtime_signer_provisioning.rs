//! Recoverable public provisioning intents. This module never handles private material.
use super::{
    StoreError, WorkflowStore, connections, new_id,
    runtime_signers::{self, SaveSigner, Signer},
    validate_key, validate_name,
};
use serde::{Deserialize, Serialize};
use tokio_postgres::{Row, Transaction};

#[derive(Clone, Debug, Serialize)]
pub struct BeginSignerProvisioning<'a> {
    pub key: &'a str,
    pub display_name: &'a str,
    pub namespace: &'a str,
    pub key_id: &'a str,
    pub mode: &'a str,
    pub import_public_key: &'a str,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignerProvisioning {
    pub id: String,
    pub idempotency_key: String,
    pub display_name: String,
    pub namespace: String,
    pub key_id: String,
    pub mode: String,
    pub import_public_key: String,
    pub secret_name: String,
    pub state: String,
    pub safe_reason: String,
    pub public_key: String,
    pub secret_uid: String,
    pub signer: Option<Signer>,
    pub accepted_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}
fn public_key(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn validate(input: &BeginSignerProvisioning<'_>) -> Result<String, StoreError> {
    validate_key(input.key)?;
    validate_name(input.display_name, "display_name")?;
    if input.display_name.len() > 120
        || input.key_id == "unsigned"
        || input.key_id.is_empty()
        || input.key_id.len() > 128
        || !input
            .key_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(StoreError::InvalidInput("signer metadata"));
    }
    if input.namespace.is_empty()
        || input.namespace.len() > 63
        || !input
            .namespace
            .starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        || !input
            .namespace
            .ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        || !input
            .namespace
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(StoreError::InvalidInput("signer namespace"));
    }
    match input.mode {
        "generate" if input.import_public_key.is_empty() => {}
        "import" if public_key(input.import_public_key) => {}
        _ => return Err(StoreError::InvalidInput("signer provisioning mode")),
    }
    serde_json::to_string(input).map_err(|_| StoreError::CorruptState)
}
fn operation(row: &Row) -> Result<SignerProvisioning, StoreError> {
    let result: &str = row.get("result");
    Ok(SignerProvisioning {
        id: row.get("id"),
        idempotency_key: row.get("idempotency_key"),
        display_name: row.get("display_name"),
        namespace: row.get("namespace"),
        key_id: row.get("key_id"),
        mode: row.get("mode"),
        import_public_key: row.get("import_public_key"),
        secret_name: row.get("secret_name"),
        state: row.get("state"),
        safe_reason: row.get("safe_reason"),
        public_key: row.get("public_key"),
        secret_uid: row.get("secret_uid"),
        signer: if result.is_empty() {
            None
        } else {
            Some(serde_json::from_str(result).map_err(|_| StoreError::CorruptState)?)
        },
        accepted_at_unix_ms: u64::try_from(row.get::<_, i64>("accepted_at_unix_ms"))
            .map_err(|_| StoreError::CorruptState)?,
        updated_at_unix_ms: u64::try_from(row.get::<_, i64>("updated_at_unix_ms"))
            .map_err(|_| StoreError::CorruptState)?,
    })
}
async fn locked(tx: &Transaction<'_>, key: &str) -> Result<SignerProvisioning, StoreError> {
    operation(&tx.query_opt("SELECT * FROM workflow_runtime_signer_provisioning WHERE idempotency_key=$1 FOR UPDATE",&[&key]).await?.ok_or(StoreError::NotFound)?)
}
pub(super) async fn reject_reserved(tx: &Transaction<'_>, key: &str) -> Result<(), StoreError> {
    if tx
        .query_opt(
            "SELECT 1 FROM workflow_runtime_signer_provisioning WHERE idempotency_key=$1",
            &[&key],
        )
        .await?
        .is_some()
    {
        return Err(StoreError::IdempotencyConflict);
    }
    Ok(())
}
impl WorkflowStore {
    /// Reserve one deterministic object name before any Kubernetes create request.
    /// # Errors
    /// Reject malformed inputs, changed retries, manual-command key collisions and database errors.
    pub async fn begin_runtime_signer_provisioning(
        &self,
        input: BeginSignerProvisioning<'_>,
    ) -> Result<SignerProvisioning, StoreError> {
        let safe = validate(&input)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, input.key).await?;
        if let Some(row) = tx
            .query_opt(
                "SELECT * FROM workflow_runtime_signer_provisioning WHERE idempotency_key=$1",
                &[&input.key],
            )
            .await?
        {
            if row.get::<_, String>("safe_request") != safe {
                return Err(StoreError::IdempotencyConflict);
            }
            let value = operation(&row)?;
            tx.commit().await?;
            driver.abort();
            return Ok(value);
        }
        if tx
            .query_opt(
                "SELECT 1 FROM workflow_runtime_signer_receipts WHERE idempotency_key=$1",
                &[&input.key],
            )
            .await?
            .is_some()
        {
            return Err(StoreError::IdempotencyConflict);
        }
        let id = new_id();
        let name = format!("ayni-runtime-signer-{}", id.replace('-', ""));
        let row=tx.query_one("INSERT INTO workflow_runtime_signer_provisioning(id,idempotency_key,safe_request,display_name,namespace,key_id,mode,import_public_key,secret_name,state,safe_reason,accepted_at_unix_ms,updated_at_unix_ms) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,'pending','',now,now FROM (SELECT (extract(epoch FROM clock_timestamp())*1000)::bigint AS now) t RETURNING *",&[&id,&input.key,&safe,&input.display_name,&input.namespace,&input.key_id,&input.mode,&input.import_public_key,&name]).await?;
        let value = operation(&row)?;
        tx.commit().await?;
        driver.abort();
        Ok(value)
    }
    /// Read safe intent and its original completed result without external effects.
    /// # Errors
    /// Reject malformed or absent keys and database errors.
    pub async fn get_runtime_signer_provisioning(
        &self,
        key: &str,
    ) -> Result<SignerProvisioning, StoreError> {
        validate_key(key)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client
            .query_opt(
                "SELECT * FROM workflow_runtime_signer_provisioning WHERE idempotency_key=$1",
                &[&key],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let value = operation(&row)?;
        driver.abort();
        Ok(value)
    }
    /// Atomically register the externally verified immutable object's public identity.
    /// The caller must verify Kubernetes existence, ownership and immutable creation.
    /// # Errors
    /// Reject absent intents, malformed identities, changed winners and database errors.
    pub async fn complete_runtime_signer_provisioning(
        &self,
        key: &str,
        secret_uid: &str,
        public: &str,
    ) -> Result<SignerProvisioning, StoreError> {
        validate_key(key)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, key).await?;
        let current = locked(&tx, key).await?;
        let input = SaveSigner {
            key,
            id: "",
            expected_revision: 0,
            display_name: &current.display_name,
            key_id: &current.key_id,
            public_key: public,
            secret_name: &current.secret_name,
            secret_uid,
        };
        runtime_signers::validate(&input)?;
        if current.state == "completed" {
            if current.secret_uid != secret_uid || current.public_key != public {
                return Err(StoreError::IdempotencyConflict);
            }
            tx.commit().await?;
            driver.abort();
            return Ok(current);
        }
        if current.mode == "import" && current.import_public_key != public {
            return Err(StoreError::IdempotencyConflict);
        }
        let value = Signer {
            id: new_id(),
            display_name: current.display_name.clone(),
            key_id: current.key_id.clone(),
            public_key: public.to_owned(),
            secret_name: current.secret_name.clone(),
            secret_uid: secret_uid.to_owned(),
            active_revision: new_id(),
            revision: 1,
            enabled: true,
        };
        tx.execute("INSERT INTO workflow_runtime_signers(id,display_name,key_id,public_key,secret_name,secret_uid,active_revision,revision,enabled) VALUES($1,$2,$3,$4,$5,$6,$7,1,true)",&[&value.id,&value.display_name,&value.key_id,&value.public_key,&value.secret_name,&value.secret_uid,&value.active_revision]).await?;
        let safe:String=tx.query_one("SELECT safe_request FROM workflow_runtime_signer_provisioning WHERE idempotency_key=$1",&[&key]).await?.get(0);
        runtime_signers::retain(&tx, key, "provision", &safe, &value).await?;
        let result = serde_json::to_string(&value).map_err(|_| StoreError::CorruptState)?;
        let row=tx.query_one("UPDATE workflow_runtime_signer_provisioning SET state='completed',safe_reason='',public_key=$2,secret_uid=$3,result=$4,updated_at_unix_ms=GREATEST(updated_at_unix_ms,(extract(epoch FROM clock_timestamp())*1000)::bigint) WHERE idempotency_key=$1 RETURNING *",&[&key,&public,&secret_uid,&result]).await?;
        let op = operation(&row)?;
        tx.commit().await?;
        driver.abort();
        Ok(op)
    }
    /// Retain a closed public recovery reason without overwriting completion.
    /// # Errors
    /// Reject unknown reasons, absent intents, malformed keys and database errors.
    pub async fn note_runtime_signer_provisioning_failure(
        &self,
        key: &str,
        reason: &str,
    ) -> Result<SignerProvisioning, StoreError> {
        validate_key(key)?;
        let state = match reason {
            "signer_provisioning_inputs_required" => "awaiting_material",
            "signer_provisioning_conflict" => "blocked",
            "signer_provisioning_disabled"
            | "signer_provisioning_unavailable"
            | "signer_provisioning_access_denied" => "pending",
            _ => return Err(StoreError::InvalidInput("signer provisioning reason")),
        };
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, key).await?;
        let value = locked(&tx, key).await?;
        if value.state == "completed" {
            tx.commit().await?;
            driver.abort();
            return Ok(value);
        }
        let row=tx.query_one("UPDATE workflow_runtime_signer_provisioning SET state=$2,safe_reason=$3,updated_at_unix_ms=GREATEST(updated_at_unix_ms,(extract(epoch FROM clock_timestamp())*1000)::bigint) WHERE idempotency_key=$1 RETURNING *",&[&key,&state,&reason]).await?;
        let value = operation(&row)?;
        tx.commit().await?;
        driver.abort();
        Ok(value)
    }
}
