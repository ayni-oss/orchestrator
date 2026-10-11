//! Repository-scoped resolution of the official catalog builder.
use super::{
    Digest, ExecutionBackend, ImageDiscovery, ProfileRuntime, RuntimeCursor, RuntimeReconciler,
    discovery_error, operation_name,
};

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    /// Resolve only the catalog-owned builder and retain its platform manifest identity.
    pub(super) async fn resolve_builder(
        &self,
        id: &str,
        version: &str,
        runtime: &mut ProfileRuntime,
        cursor: &mut RuntimeCursor,
    ) -> Result<bool, &'static str> {
        let contract = ayni_orchestrator_profile_ayni::compatibility::contract(version)?;
        let key = cursor
            .preparation
            .as_ref()
            .ok_or("runtime_preparation_missing")?
            .generation
            .clone();
        if cursor.builder.contract.as_ref() == Some(&key)
            && let Some(image) = cursor.builder.image.as_ref()
        {
            runtime
                .build
                .as_mut()
                .ok_or("runtime_builder_missing")?
                .image
                .clone_from(image);
            if let Some(name) = cursor.builder.job.as_ref() {
                self.kubernetes.cleanup(name).await?;
                cursor.builder.job = None;
                cursor.builder.started_at = None;
                self.save(id, cursor).await?;
            }
            return Ok(true);
        }
        let name = operation_name(&format!("official-builder:{id}:{key}"), cursor.sequence);
        if cursor.builder.job.as_ref() != Some(&name) || cursor.builder.started_at.is_none() {
            cursor.builder.started_at = Some(super::now()?);
        }
        cursor.builder.job = Some(name.clone());
        cursor
            .status
            .insert("phase".into(), "resolving_builder".into());
        self.save(id, cursor).await?;
        let mut discovery = runtime.execution_runtime();
        discovery.image = contract.builder_reference.into();
        discovery.build = None;
        discovery.pull_secret = None;
        self.kubernetes.start_discovery(&name, &discovery).await?;
        let image = match self.kubernetes.discover(&name).await? {
            ImageDiscovery::Pending => {
                let started = cursor
                    .builder
                    .started_at
                    .ok_or("runtime_preparation_invalid")?;
                if super::now()?.saturating_sub(started) > 180 {
                    return Err("official_builder_timeout");
                }
                return Ok(false);
            }
            ImageDiscovery::Missing => return Err("official_builder_unavailable"),
            ImageDiscovery::Failed { reason } => return Err(discovery_error(&reason)),
            ImageDiscovery::Resolved { digest, .. } => {
                if !ayni_orchestrator_domain::digest::digest(&digest) {
                    return Err("official_builder_digest_invalid");
                }
                format!("ghcr.io/ayni-oss/ayni-builder@{digest}")
            }
        };
        runtime
            .build
            .as_mut()
            .ok_or("runtime_builder_missing")?
            .image
            .clone_from(&image);
        cursor.builder.contract = Some(key);
        cursor.builder.image = Some(image);
        if let Some(preparation) = &mut cursor.preparation {
            preparation.runtime = runtime.clone();
        }
        self.save(id, cursor).await?;
        self.kubernetes.cleanup(&name).await?;
        cursor.builder.job = None;
        cursor.builder.started_at = None;
        self.save(id, cursor).await?;
        Ok(true)
    }
}

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    #[allow(
        clippy::too_many_arguments,
        reason = "freeze all source and infrastructure observations as one generation"
    )]
    pub(super) async fn freeze_preparation(
        &self,
        id: &str,
        runtime: &ProfileRuntime,
        head: &str,
        policy: &[u8],
        lock: &[u8],
        lock_identity: &crate::profiles::ayni::environment::LockIdentity,
        registry: &super::v1::RuntimeRegistrySelection,
        certificate: Option<&super::SigningIdentity>,
        cursor: &mut RuntimeCursor,
    ) -> Result<bool, &'static str> {
        if let Some(pending) = &cursor.pending {
            let mut desired = self.configured_profile(id).await?;
            if let Some(build) = &mut desired.build {
                build.image.clone_from(
                    &runtime
                        .build
                        .as_ref()
                        .ok_or("runtime_builder_missing")?
                        .image,
                );
                build.signing_secret.clone_from(
                    &certificate
                        .as_ref()
                        .ok_or("certificate_setup_required")?
                        .secret_name,
                );
            }
            let (_, desired_digest) =
                super::normalized_runtime_source(&desired, &lock_identity.fingerprint)?;
            let desired_digest = super::certificate_source_digest(&desired_digest, certificate)?;
            if desired_digest != pending.source_digest
                || pending.request.revision != head
                || cursor.preparation.as_ref().is_some_and(|identity| {
                    identity.policy_digest != format!("sha256:{:x}", super::Sha256::digest(policy))
                        || identity.lock_digest
                            != format!("sha256:{:x}", super::Sha256::digest(lock))
                })
            {
                self.cleanup_operation(pending).await?;
                cursor.pending = None;
                cursor.status.insert("phase".into(), "analyzing".into());
                self.save(id, cursor).await?;
                return Ok(false);
            }
        }
        if cursor.pending.is_none() {
            let policy_digest = format!("sha256:{:x}", super::Sha256::digest(policy));
            let lock_digest = format!("sha256:{:x}", super::Sha256::digest(lock));
            let generation = format!(
                "{:x}",
                super::Sha256::digest(
                    serde_json::to_vec(&(
                        &head,
                        &policy_digest,
                        &lock_digest,
                        &runtime,
                        &prost::Message::encode_to_vec(registry),
                        &certificate,
                    ))
                    .map_err(|_| "runtime_preparation_invalid")?
                )
            );
            if cursor
                .preparation
                .as_ref()
                .is_none_or(|identity| identity.generation != generation)
            {
                if let Some(name) = cursor.builder.job.as_ref() {
                    self.kubernetes.cleanup(name).await?;
                }
                cursor.failures = 0;
                cursor.accounting.blocked = false;
                cursor.next_at = 0;
                cursor.status.remove("reason-code");
                cursor.status.remove("error-class");
                cursor.status.remove("last-error");
                cursor.preparation = Some(super::PreparationIdentity {
                    generation,
                    source_revision: head.to_owned(),
                    policy_digest,
                    lock_digest,
                    contract_version: lock_identity.ayni_version.clone(),
                    platform: runtime.platform.clone(),
                    registry: prost::Message::encode_to_vec(registry),
                    certificate: certificate.cloned(),
                    runtime: runtime.clone(),
                });
                self.save(id, cursor).await?;
            }
        }
        Ok(true)
    }
}

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn repository_inputs(
        &self,
        github: &super::v1::GithubRepository,
        head: &str,
        token: &str,
    ) -> Result<(Vec<u8>, Vec<u8>), &'static str> {
        let policy = self
            .source
            .file(&github.owner, &github.name, head, ".ayni.toml", token)
            .await
            .map_err(|reason| {
                if reason == "github_not_found" {
                    "policy_missing"
                } else {
                    reason
                }
            })?;
        if policy.is_empty() {
            return Err("policy_missing");
        }
        let lock = self
            .source
            .lock(&github.owner, &github.name, head, token)
            .await
            .map_err(|reason| {
                if reason == "github_not_found" {
                    "lock_missing"
                } else {
                    reason
                }
            })?;
        Ok((policy, lock))
    }
}
