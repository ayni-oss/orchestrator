//! Durable one-shot onboarding. Grants apply to one run; the browser is not its worker.
use super::{StoreError, WorkflowStore, connections, new_id, validate_id, validate_key};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    pub repository_id: String,
    pub expected_connection_revision: u64,
    pub registry_connection_id: String,
    pub signer_id: String,
    pub platform: String,
    pub max_seconds: u32,
    pub max_builds: u32,
    pub allow_build: bool,
    pub allow_replace: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Run {
    pub id: String,
    pub idempotency_key: String,
    pub settings: Settings,
    pub registry_revision: u64,
    pub signer_revision: u64,
    pub state: String,
    pub safe_reason: String,
    pub commit_sha: String,
    pub inspection_id: String,
    pub preparation_id: String,
    pub inspection_key: String,
    pub preparation_key: String,
}
pub struct Claim {
    pub run: Run,
    pub token: String,
}
fn decode(value: &str) -> Result<Run, StoreError> {
    serde_json::from_str(value).map_err(|_| StoreError::CorruptState)
}
fn encode<T: Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|_| StoreError::CorruptState)
}
fn validate_settings(settings: &Settings) -> Result<(), StoreError> {
    for id in [
        &settings.repository_id,
        &settings.registry_connection_id,
        &settings.signer_id,
    ] {
        validate_id(id, "runtime_configuration_id")?;
    }
    if !matches!(settings.platform.as_str(), "linux/arm64" | "linux/amd64")
        || !(1..=86400).contains(&settings.max_seconds)
        || settings.max_builds > 10
        || settings.allow_build != (settings.max_builds > 0)
        || settings.allow_replace && !settings.allow_build
    {
        return Err(StoreError::InvalidInput("runtime_allowance"));
    }
    Ok(())
}
impl WorkflowStore {
    /// Retain reviewed settings and original receipt, serializing starts per repository.
    /// # Errors
    /// Reject invalid bounds, reused keys, stale/disabled configuration or overlapping work.
    pub async fn start_repository_runtime(
        &self,
        key: &str,
        settings: Settings,
    ) -> Result<Run, StoreError> {
        validate_key(key)?;
        validate_settings(&settings)?;
        let safe = encode(&settings)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, key).await?;
        if let Some(row)=tx.query_opt("SELECT safe_request,receipt FROM workflow_repository_runtime_runs WHERE idempotency_key=$1",&[&key]).await? {
            if row.get::<_,String>("safe_request")!=safe { return Err(StoreError::IdempotencyConflict); }
            let run=decode(&row.get::<_,String>("receipt"))?;tx.commit().await?;driver.abort();return Ok(run);
        }
        connections::serialize_key(&tx, &settings.repository_id).await?;
        if tx.query_opt("SELECT id FROM workflow_repository_runtime_runs WHERE repository_id=$1 AND state IN ('resolving','inspecting','admitting')",&[&settings.repository_id]).await?.is_some()
            || tx.query_opt("SELECT id FROM workflow_runtime_preparations WHERE repository_id=$1 AND (state IN ('pending','running','stopping','cancelling','cleanup_pending','publication_uncertain') OR NOT (evidence::jsonb->>'cleanup_complete')::boolean)",&[&settings.repository_id]).await?.is_some() {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        let config=tx.query_opt("SELECT c.revision FROM workflow_repositories r JOIN workflow_connections c ON c.id=r.connection_id WHERE r.id=$1 AND c.enabled AND c.active_credential_revision<>'' FOR SHARE OF c",&[&settings.repository_id]).await?.ok_or(StoreError::ConfigurationRevisionConflict)?;
        if u64::try_from(config.get::<_, i64>(0)).ok()
            != Some(settings.expected_connection_revision)
        {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        let registry=tx.query_opt("SELECT revision FROM workflow_connections WHERE id=$1 AND capability='container_registry' AND enabled AND active_credential_revision<>'' FOR SHARE",&[&settings.registry_connection_id]).await?.ok_or(StoreError::ConfigurationRevisionConflict)?;
        let signer = tx
            .query_opt(
                "SELECT revision FROM workflow_runtime_signers WHERE id=$1 AND enabled FOR SHARE",
                &[&settings.signer_id],
            )
            .await?
            .ok_or(StoreError::ConfigurationRevisionConflict)?;
        let run = Run {
            id: new_id(),
            idempotency_key: key.into(),
            settings,
            registry_revision: u64::try_from(registry.get::<_, i64>(0))
                .map_err(|_| StoreError::CorruptState)?,
            signer_revision: u64::try_from(signer.get::<_, i64>(0))
                .map_err(|_| StoreError::CorruptState)?,
            state: "resolving".into(),
            safe_reason: String::new(),
            commit_sha: String::new(),
            inspection_id: String::new(),
            preparation_id: String::new(),
            inspection_key: new_id(),
            preparation_key: new_id(),
        };
        let encoded = encode(&run)?;
        tx.execute("INSERT INTO workflow_repository_runtime_runs(id,idempotency_key,repository_id,safe_request,receipt,state,progress) VALUES($1,$2,$3,$4,$5,'resolving',$5)",&[&run.id,&key,&run.settings.repository_id,&safe,&encoded]).await?;
        tx.commit().await?;
        driver.abort();
        Ok(run)
    }
    /// Load the latest run, including its reviewed settings, without starting work.
    /// # Errors
    /// Reject invalid IDs or database/retained-state errors.
    pub async fn latest_repository_runtime(
        &self,
        repository_id: &str,
    ) -> Result<Option<Run>, StoreError> {
        validate_id(repository_id, "repository_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row=client.query_opt("SELECT progress FROM workflow_repository_runtime_runs WHERE repository_id=$1 ORDER BY id DESC LIMIT 1",&[&repository_id]).await?;
        driver.abort();
        row.map(|r| decode(&r.get::<_, String>(0))).transpose()
    }
    /// Recover the original acknowledgement, never a later mutable projection.
    /// # Errors
    /// Reject invalid keys, absent receipts and database errors.
    pub async fn repository_runtime_receipt(&self, key: &str) -> Result<Run, StoreError> {
        validate_key(key)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client
            .query_opt(
                "SELECT receipt FROM workflow_repository_runtime_runs WHERE idempotency_key=$1",
                &[&key],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        driver.abort();
        decode(&row.get::<_, String>(0))
    }
    /// Claim one bounded step; another process may recover it after lease expiry.
    /// # Errors
    /// Return database or retained-state errors.
    pub async fn claim_repository_runtime(&self) -> Result<Option<Claim>, StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        let token = new_id();
        let row=client.query_opt("UPDATE workflow_repository_runtime_runs SET claim_token=$1,lease_until=clock_timestamp()+interval '60 seconds' WHERE id=(SELECT id FROM workflow_repository_runtime_runs WHERE state IN ('resolving','inspecting','admitting') AND lease_until<clock_timestamp() ORDER BY id FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING progress",&[&token]).await?;
        driver.abort();
        row.map(|r| {
            Ok(Claim {
                run: decode(&r.get::<_, String>(0))?,
                token,
            })
        })
        .transpose()
    }
    /// Check configuration revisions immediately before each external admission step.
    /// # Errors
    /// Reject disabled or changed public configuration.
    pub async fn validate_repository_runtime_configuration(
        &self,
        run: &Run,
    ) -> Result<(), StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        let valid:bool=client.query_one("SELECT EXISTS(SELECT 1 FROM workflow_repositories r JOIN workflow_connections c ON c.id=r.connection_id WHERE r.id=$1 AND c.enabled AND c.active_credential_revision<>'' AND c.revision=$2) AND EXISTS(SELECT 1 FROM workflow_connections WHERE id=$3 AND enabled AND active_credential_revision<>'' AND revision=$4) AND EXISTS(SELECT 1 FROM workflow_runtime_signers WHERE id=$5 AND enabled AND revision=$6)",&[&run.settings.repository_id,&i64::try_from(run.settings.expected_connection_revision).map_err(|_|StoreError::CorruptState)?,&run.settings.registry_connection_id,&i64::try_from(run.registry_revision).map_err(|_|StoreError::CorruptState)?,&run.settings.signer_id,&i64::try_from(run.signer_revision).map_err(|_|StoreError::CorruptState)?]).await?.get(0);
        driver.abort();
        if valid {
            Ok(())
        } else {
            Err(StoreError::ConfigurationRevisionConflict)
        }
    }
    /// Advance only the live claim. Frozen settings, command keys and receipts cannot change.
    /// # Errors
    /// Reject stale workers, invalid transitions and database errors.
    pub async fn advance_repository_runtime(
        &self,
        claim: &Claim,
        state: &str,
        value: &str,
    ) -> Result<(), StoreError> {
        let mut run = claim.run.clone();
        match (run.state.as_str(), state) {
            ("resolving", "inspecting")
                if value.len() == 40
                    && value
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) =>
            {
                run.commit_sha = value.into();
            }
            ("inspecting", "admitting") => {
                validate_id(value, "inspection_id")?;
                run.inspection_id = value.into();
            }
            ("admitting", "completed") => {
                validate_id(value, "preparation_id")?;
                run.preparation_id = value.into();
            }
            ("resolving" | "inspecting" | "admitting", "blocked")
                if matches!(
                    value,
                    "source_unavailable"
                        | "configuration_changed"
                        | "inspection_failed"
                        | "inspection_findings"
                        | "runtime_profile_unavailable"
                        | "preparation_admission_failed"
                        | "step_unavailable"
                ) =>
            {
                run.safe_reason = value.into();
            }
            _ => return Err(StoreError::InvalidInput("runtime_onboarding_transition")),
        }
        run.state = state.into();
        let encoded = encode(&run)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let count=client.execute("UPDATE workflow_repository_runtime_runs SET state=$1,progress=$2,claim_token='',lease_until='-infinity' WHERE id=$3 AND claim_token=$4 AND state=$5 AND lease_until>clock_timestamp() AND ($1<>'admitting' OR EXISTS(SELECT 1 FROM workflow_repository_inspections i WHERE i.id=$6 AND i.idempotency_key=workflow_repository_runtime_runs.progress::jsonb->>'inspection_key' AND i.commit_sha=workflow_repository_runtime_runs.progress::jsonb->>'commit_sha' AND i.state='completed')) AND ($1<>'completed' OR EXISTS(SELECT 1 FROM workflow_runtime_preparations p WHERE p.id=$6 AND p.idempotency_key=workflow_repository_runtime_runs.progress::jsonb->>'preparation_key'))",&[&state,&encoded,&run.id,&claim.token,&claim.run.state,&value]).await?;
        driver.abort();
        if count == 1 {
            Ok(())
        } else {
            Err(StoreError::ConfigurationRevisionConflict)
        }
    }
}
