use super::*;
use crate::store::repository_inspections::InspectionResult;
impl WorkflowStore {
    /// Replay a public start receipt before consulting providers or current deployment configuration.
    /// # Errors
    /// Reject changed requests under an existing key or unavailable retained state.
    pub async fn replay_runtime_start(
        &self,
        input: &Start<'_>,
    ) -> Result<Option<Operation>, StoreError> {
        validation::start(input)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let result = receipt(&tx, input.key, "start", &digest(json(input)?)).await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Freeze validated exact-source inputs and all current capability revisions before scheduling work.
    /// Official policy/trust interpretation belongs to the trusted acquisition caller.
    /// # Errors
    /// Reject stale capabilities, incomplete inspection, altered source bytes, unqualified profiles or reused keys.
    pub async fn start_runtime_preparation(
        &self,
        input: Start<'_>,
        freeze: FreezeInputs,
    ) -> Result<Operation, StoreError> {
        validation::start(&input)?;
        let request_digest = digest(json(&input)?);
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, input.key).await?;
        if let Some(previous) = receipt(&tx, input.key, "start", &request_digest).await? {
            tx.commit().await?;
            driver.abort();
            return Ok(previous);
        }
        validation::frozen(&freeze, &input)?;
        connections::serialize_key(&tx, input.repository_id).await?;
        let id = new_id();
        let at = now(&tx).await?;
        let frozen = load_snapshot(&tx, &input, freeze, &id).await?;
        let encoded = json(&frozen)?;
        let frozen_digest = digest(&encoded);
        let deadline = integer(at + u64::from(input.max_seconds) * 1000)?;
        let row=tx.query_one("INSERT INTO workflow_runtime_preparations(id,idempotency_key,request_digest,repository_id,inspection_id,registry_connection_id,signer_id,platform,frozen,frozen_input_digest,state,stage,generation,allow_build,allow_replace,max_builds,deadline_unix_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'pending','acquire_source',1,$11,$12,$13,$14) RETURNING *",&[&id,&input.key,&request_digest,&input.repository_id,&input.inspection_id,&input.registry_connection_id,&input.signer_id,&input.platform,&encoded,&frozen_digest,&input.allow_build,&input.allow_replace,&i32::try_from(input.max_builds).map_err(|_|StoreError::InvalidInput("max_builds"))?,&deadline]).await?;
        // Only this desired identity may become ready; older active work is explicitly stopped and cleaned.
        tx.execute("UPDATE workflow_runtime_preparations SET state='cancelling',terminal_state='superseded',safe_reason='selection_superseded',updated_at_unix_ms=$2 WHERE repository_id=$1 AND id<>$3 AND state IN ('pending','running','stopping','cleanup_pending','unavailable','failed','blocked','publication_uncertain')",&[&input.repository_id,&integer(at)?,&id]).await?;
        tx.execute("INSERT INTO workflow_runtime_preparation_selections(repository_id,operation_id) VALUES($1,$2) ON CONFLICT(repository_id) DO UPDATE SET operation_id=EXCLUDED.operation_id",&[&input.repository_id,&id]).await?;
        let result = operation(&row)?;
        save_receipt(&tx, input.key, "start", &request_digest, &result).await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Cancel through the retained generation; cleanup never implies runtime readiness.
    /// # Errors
    /// Reject stale generations or changed retries and unavailable state.
    pub async fn cancel_runtime_preparation(
        &self,
        key: &str,
        id: &str,
        generation: u64,
    ) -> Result<Operation, StoreError> {
        self.runtime_command(key, id, generation, "cancel").await
    }
    /// Reconcile the same frozen operation without extending its deadline or build allowance.
    /// # Errors
    /// Reject stale or exhausted operations and changed retry commands.
    pub async fn retry_runtime_preparation(
        &self,
        key: &str,
        id: &str,
        generation: u64,
    ) -> Result<Operation, StoreError> {
        self.runtime_command(key, id, generation, "retry").await
    }
    async fn runtime_command(
        &self,
        key: &str,
        id: &str,
        generation: u64,
        kind: &str,
    ) -> Result<Operation, StoreError> {
        validate_key(key)?;
        validate_id(id, "operation_id")?;
        let request_digest = digest(json(&(id, generation, kind))?);
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        connections::serialize_key(&tx, key).await?;
        if let Some(previous) = receipt(&tx, key, kind, &request_digest).await? {
            tx.commit().await?;
            driver.abort();
            return Ok(previous);
        }
        let op = locked(&tx, id).await?;
        if op.generation != generation {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        let at = now(&tx).await?;
        if kind == "cancel" {
            if !matches!(op.state.as_str(), "ready" | "cancelled" | "superseded") {
                tx.execute("UPDATE workflow_runtime_preparations SET state='cancelling',terminal_state='cancelled',safe_reason='cancelled',updated_at_unix_ms=$2 WHERE id=$1",&[&id,&integer(at)?]).await?;
            }
        } else if matches!(
            op.state.as_str(),
            "unavailable" | "failed" | "blocked" | "publication_uncertain" | "cleanup_pending"
        ) {
            if at > op.deadline_unix_ms {
                return Err(StoreError::Capacity("runtime deadline exhausted"));
            }
            if !op.evidence.cleanup_complete {
                return Err(StoreError::InvalidInput("runtime cleanup incomplete"));
            }
            if !current(&tx, &op).await? {
                return Err(StoreError::ConfigurationRevisionConflict);
            }
            let mut evidence = op.evidence.clone();
            evidence.cleanup_complete = false;
            evidence.content_verified = false;
            evidence.managed_compatible = false;
            tx.execute("UPDATE workflow_runtime_preparations SET state='pending',stage='acquire_source',terminal_state='',safe_reason='',evidence=$3,generation=generation+1,updated_at_unix_ms=$2 WHERE id=$1",&[&id,&integer(at)?,&json(&evidence)?]).await?;
        }
        let result = locked(&tx, id).await?;
        save_receipt(&tx, key, kind, &request_digest, &result).await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
}
/// Current desired selection and all capability references must still equal their frozen values.
pub(super) async fn current(tx: &Transaction<'_>, op: &Operation) -> Result<bool, StoreError> {
    let f = &op.frozen;
    let row=tx.query_opt("SELECT 1 FROM workflow_runtime_preparation_selections s JOIN workflow_connections src ON src.id=$3 JOIN workflow_connections reg ON reg.id=$5 JOIN workflow_runtime_signers signer ON signer.id=$7 WHERE s.repository_id=$1 AND s.operation_id=$2 AND src.enabled AND src.revision=$4 AND src.active_credential_revision=$9 AND reg.enabled AND reg.revision=$6 AND reg.active_credential_revision=$10 AND signer.enabled AND signer.revision=$8 AND signer.active_revision=$11 AND signer.secret_uid=$12 FOR SHARE OF s,src,reg,signer",&[&op.repository_id,&op.id,&f.source_connection_id,&integer(f.source_connection_revision)?,&f.registry_connection_id,&integer(f.registry_connection_revision)?,&f.signer_id,&integer(f.signer_revision)?,&f.source_credential_revision,&f.registry_credential_revision,&f.signer_active_revision,&f.signer_secret_uid]).await?;
    Ok(row.is_some())
}

struct AdmissionSnapshot {
    repository: Row,
    source: connections::Connection,
    registry: connections::Connection,
    signer: Row,
    inspection: Row,
    result: InspectionResult,
    reusable_image: String,
}
impl AdmissionSnapshot {
    fn freeze(
        self,
        input: &Start<'_>,
        freeze: FreezeInputs,
        id: &str,
    ) -> Result<FrozenInputs, StoreError> {
        let Self {
            repository,
            source,
            registry,
            signer,
            inspection,
            result,
            reusable_image,
        } = self;
        let key_id: String = signer.get("key_id");
        let discovery_tag = format!(
            "ayni-{}-{}",
            result.lock_fingerprint.trim_start_matches("sha256:"),
            input.platform.trim_start_matches("linux/")
        );
        Ok(FrozenInputs {
            repository_id: input.repository_id.into(),
            inspection_id: input.inspection_id.into(),
            source_commit: inspection.get("commit_sha"),
            source_tree: freeze.source_tree,
            source_connection_id: source.id,
            source_connection_revision: source.revision,
            source_credential_revision: source.active_credential_revision,
            installation_id: unsigned(&repository, "installation_id")?,
            provider_repository_id: unsigned(&repository, "provider_repository_id")?,
            repository_full_name: repository.get("full_name"),
            registry_connection_id: registry.id,
            registry_connection_revision: registry.revision,
            registry_credential_revision: registry.active_credential_revision,
            registry_destination: registry_destination(
                &registry.settings.registry_destination,
                repository.get("full_name"),
            )?,
            discovery_tag,
            reusable_image,
            publication_tag: format!("ayni-preparation-{}", id.replace('-', "")),
            signer_id: input.signer_id.into(),
            signer_revision: unsigned(&signer, "revision")?,
            signer_active_revision: signer.get("active_revision"),
            signer_key_id: key_id,
            signer_public_key: signer.get("public_key"),
            signer_secret_name: signer.get("secret_name"),
            signer_secret_uid: signer.get("secret_uid"),
            policy: freeze.policy,
            lock: freeze.lock,
            policy_digest: result.policy_digest,
            lock_digest: result.lock_digest,
            lock_fingerprint: result.lock_fingerprint,
            sources: freeze.sources,
            profile_json: freeze.profile_json,
        })
    }
}

fn validate_inspection(
    result: &InspectionResult,
    freeze: &FreezeInputs,
    key_id: &str,
) -> Result<(), StoreError> {
    if result.policy_digest != digest(freeze.policy.as_bytes())
        || result.lock_digest != digest(freeze.lock.as_bytes())
        || !validation::sha(&result.lock_fingerprint)
        || !result
            .trust_key_ids
            .iter()
            .any(|candidate| candidate == key_id)
        || result.findings.iter().any(|v| {
            !matches!(
                v.code.as_str(),
                "adapter_discovery_pending" | "runtime_verification_pending"
            )
        })
    {
        return Err(StoreError::InvalidInput("runtime inspection prerequisite"));
    }
    Ok(())
}

async fn load_snapshot(
    tx: &Transaction<'_>,
    input: &Start<'_>,
    freeze: FreezeInputs,
    id: &str,
) -> Result<FrozenInputs, StoreError> {
    let repository = tx
        .query_opt(
            "SELECT * FROM workflow_repositories WHERE id=$1",
            &[&input.repository_id],
        )
        .await?
        .ok_or(StoreError::NotFound)?;
    let source_id: String = repository.get("connection_id");
    let source = connections::locked(tx, &source_id).await?;
    let registry = connections::locked(tx, input.registry_connection_id).await?;
    let signer = tx
        .query_opt(
            "SELECT * FROM workflow_runtime_signers WHERE id=$1 FOR UPDATE",
            &[&input.signer_id],
        )
        .await?
        .ok_or(StoreError::NotFound)?;
    validate_capabilities(&source, &registry, &signer, freeze.expected_signer_revision)?;
    let inspection = tx
        .query_opt(
            "SELECT * FROM workflow_repository_inspections WHERE id=$1 FOR SHARE",
            &[&input.inspection_id],
        )
        .await?
        .ok_or(StoreError::NotFound)?;
    if inspection.get::<_, String>("repository_id") != input.repository_id
        || inspection.get::<_, String>("state") != "completed"
        || inspection.get::<_, String>("connection_id") != source.id
        || unsigned(&inspection, "connection_revision")? != source.revision
        || inspection.get::<_, String>("credential_revision") != source.active_credential_revision
    {
        return Err(StoreError::ConfigurationRevisionConflict);
    }
    let result: InspectionResult =
        serde_json::from_str(inspection.get("result")).map_err(|_| StoreError::CorruptState)?;
    let key_id: String = signer.get("key_id");
    validate_inspection(&result, &freeze, &key_id)?;
    let destination = registry_destination(
        &registry.settings.registry_destination,
        repository.get("full_name"),
    )?;
    let reusable=tx.query_opt("SELECT evidence FROM workflow_runtime_preparations WHERE repository_id=$1 AND state='ready' AND platform=$2 AND frozen::jsonb->>'lock_fingerprint'=$3 AND frozen::jsonb->>'registry_destination'=$4 ORDER BY id DESC LIMIT 1",&[&input.repository_id,&input.platform,&result.lock_fingerprint,&destination]).await?;
    let reusable_image = match reusable {
        Some(row) => {
            serde_json::from_str::<Evidence>(row.get("evidence"))
                .map_err(|_| StoreError::CorruptState)?
                .candidate_image
        }
        None => String::new(),
    };
    AdmissionSnapshot {
        repository,
        source,
        registry,
        signer,
        inspection,
        result,
        reusable_image,
    }
    .freeze(input, freeze, id)
}

fn validate_capabilities(
    source: &connections::Connection,
    registry: &connections::Connection,
    signer: &Row,
    expected_signer_revision: u64,
) -> Result<(), StoreError> {
    if unsigned(signer, "revision")? != expected_signer_revision
        || !source.enabled
        || source.provider != "github"
        || source.capability != "source_control"
        || source.active_credential_revision.is_empty()
        || !registry.enabled
        || registry.provider != "github"
        || registry.capability != "container_registry"
        || registry.active_credential_revision.is_empty()
        || !signer.get::<_, bool>("enabled")
    {
        return Err(StoreError::ConfigurationRevisionConflict);
    }
    Ok(())
}

/// Freeze the repository package beneath the selected organization.
fn registry_destination(namespace: &str, full_name: &str) -> Result<String, StoreError> {
    connections::registry_namespace(namespace)?;
    let (owner, name) = full_name
        .split_once('/')
        .ok_or(StoreError::InvalidInput("repository_full_name"))?;
    if owner.is_empty()
        || name.is_empty()
        || name.len() > 100
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name.as_bytes()[name.len() - 1].is_ascii_alphanumeric()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(StoreError::InvalidInput("repository image package"));
    }
    Ok(format!("{namespace}/{}", name.to_ascii_lowercase()))
}

#[cfg(test)]
mod destination_tests {
    #[test]
    fn package_comes_from_the_repository_not_the_connection() -> Result<(), crate::StoreError> {
        assert_eq!(
            super::registry_destination("ghcr.io/ayni-oss", "source-team/Sandbox")?,
            "ghcr.io/ayni-oss/sandbox"
        );
        assert_eq!(
            super::registry_destination("ghcr.io/ayni-oss", "source-team/Another-Repo")?,
            "ghcr.io/ayni-oss/another-repo"
        );
        for name in [
            "sandbox",
            "owner/",
            "owner/../escape",
            "owner/repo:tag",
            "owner/repo@digest",
        ] {
            assert!(super::registry_destination("ghcr.io/ayni-oss", name).is_err());
        }
        assert!(super::registry_destination("ghcr.io/ayni-oss/package", "owner/repo").is_err());
        Ok(())
    }
}
