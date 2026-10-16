//! Periodic, restartable managed runtime verification.
use crate::{
    AppState,
    adapters::github::SourceClient,
    process::runtime_configuration::{ProfileRuntime, RuntimeConfiguration},
};
use ayni_orchestrator_api::v1;
use ayni_orchestrator_application::execution::ExecutionBackend;
use ayni_orchestrator_application::execution::{ImageDiscovery, Observation};
use ayni_orchestrator_application::runtime_reconciliation::{
    PendingPhase, SourceIdentity, VerificationAction, pending_phase, retry_delay,
    verification_action,
};
use ayni_orchestrator_kubernetes::execution::{RuntimeKubernetes, operation_name};
use ayni_orchestrator_profile_ayni::{
    managed_runtime::RuntimeExpectation, runtime_execution::ExecutionRequest,
};
#[cfg(test)]
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::Mutex, task::JoinSet};

use crate::profiles::ayni::preparation_state::{
    PendingVerification, PreparationIdentity, RuntimeCursor, RuntimeCursorExt, SigningIdentity,
    publication_budget,
};
use ayni_orchestrator_application::runtime_preparation::ImagePreparationPhase;

struct ProfileObservation<'a> {
    id: &'a str,
    runtime: &'a ProfileRuntime,
    time: u64,
    github: v1::GithubRepository,
    head: String,
    token: String,
    source_digest: String,
    default_branch: String,
    selected_revision: Option<String>,
    registry: v1::RuntimeRegistrySelection,
    certificate: Option<SigningIdentity>,
}

pub struct RuntimeReconciler<E = RuntimeKubernetes> {
    state: AppState,
    configuration: RuntimeConfiguration,
    source: Arc<SourceClient>,
    kubernetes: E,
    pass: Mutex<()>,
    coding_pass: Mutex<()>,
    credential_refresh: Mutex<BTreeMap<String, u64>>,
}

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    async fn with_database<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut crate::adapters::sqlite::CleanStore) -> Result<T, &'static str>
        + Send
        + 'static,
    ) -> Result<T, &'static str> {
        self.state
            .database
            .call(operation)
            .await
            .map_err(|_| "runtime_store_unavailable")?
    }

    async fn configured_profiles(&self) -> Result<BTreeMap<String, ProfileRuntime>, &'static str> {
        let mut profiles = self.configuration.profiles.clone();
        if self.configuration.managed_runtime.is_none() {
            return Ok(profiles);
        }
        let configuration = self.configuration.clone();
        self.with_database(move |store| {
            let mut page = v1::PageRequest {
                page_size: 100,
                page_token: String::new(),
            };
            for _ in 0..100 {
                let result = store
                    .list_runtime_profiles(
                        None,
                        Some(v1::RuntimeProfileKind::AyniManaged as i32),
                        None,
                        Some(&page),
                    )
                    .map_err(|_| "runtime_profile_missing")?;
                for profile in result.items {
                    if let Some(runtime) = configured_managed_profile(
                        store,
                        &configuration,
                        &profile,
                        profiles.contains_key(&profile.id),
                    )? {
                        profiles.insert(profile.id, runtime);
                    }
                }
                page.page_token = result.next_page_token;
                if page.page_token.is_empty() {
                    return Ok(profiles);
                }
            }
            Err("runtime_profile_limit_exceeded")
        })
        .await
    }

    async fn registry_ready(&self, id: &str) -> Result<(), &'static str> {
        let id = id.to_owned();
        self.with_database(move |store| {
            let connection = store
                .get_connection(&id)
                .map_err(|_| "registry_connection_missing")?;
            if !connection.enabled || connection.status != "ready" {
                return Err("registry_connection_not_ready");
            }
            Ok(())
        })
        .await
    }

    async fn configured_profile(&self, id: &str) -> Result<ProfileRuntime, &'static str> {
        self.configured_profiles()
            .await?
            .remove(id)
            .ok_or("runtime_source_missing")
    }
    /// Reconciles coding separately from provider publication and image builds.
    pub async fn reconcile_coding(&self) {
        let Ok(_guard) = self.coding_pass.try_lock() else {
            return;
        };
        for profile in self.configured_profiles().await.unwrap_or_default().keys() {
            let result = tokio::time::timeout(
                Duration::from_secs(self.configuration.request_timeout_seconds * 8),
                Box::pin(self.coding(profile)),
            )
            .await
            .unwrap_or(Err("coding_observation_timeout"));
            if let Err(reason) = result {
                tracing::warn!(reason_code = reason, "coding reconciliation deferred");
            }
        }
    }

    /// Initializes opt-in runtime execution from an operator configuration file.
    ///
    /// # Errors
    /// Rejects invalid configuration or unavailable provider authentication.
    /// Runs a bounded pass without holding the database lock across network I/O.
    pub async fn reconcile(self: &Arc<Self>) {
        let Ok(_guard) = self.pass.try_lock() else {
            return;
        };
        // Removal waits for in-flight runtime observations before retiring their inputs.
        let _lifecycle = self.state.repository_lifecycle.read().await;
        let Ok(removed) = self
            .state
            .database
            .execute(|store| store.removed_runtime_cleanup())
            .await
        else {
            return;
        };
        for (name, cursor) in removed {
            if self.cleanup_exhausted_cursor(&cursor).await.is_err() {
                tracing::warn!("Removed runtime cleanup deferred");
                continue;
            }
            if self
                .state
                .database
                .execute(move |store| store.complete_removed_runtime_cleanup(&name))
                .await
                .is_err()
            {
                return;
            }
        }
        // Installation operations are disabled by migration before these cleanup intents run.
        let Ok(names) = self
            .state
            .database
            .execute(|store| store.retired_builder_cleanup())
            .await
        else {
            return;
        };
        let had_retired_operations = !names.is_empty();
        for name in names {
            if self.kubernetes.cleanup(&name).await.is_err() {
                return;
            }
            if self
                .state
                .database
                .execute(move |store| store.complete_retired_builder_cleanup(&name))
                .await
                .is_err()
            {
                return;
            }
        }
        if had_retired_operations {
            return;
        }
        let mut tasks = JoinSet::new();
        for profile in self.configured_profiles().await.unwrap_or_default().keys() {
            while tasks.len() >= self.configuration.concurrency {
                let _ = tasks.join_next().await;
            }
            let reconciler = self.clone();
            let profile = profile.clone();
            tasks.spawn(async move {
                let result = tokio::time::timeout(
                    Duration::from_secs(reconciler.configuration.request_timeout_seconds * 8),
                    reconciler.profile(&profile),
                )
                .await;
                let reason = match result {
                    Ok(Ok(())) => return,
                    Ok(Err(reason)) => reason,
                    Err(_) => "runtime_observation_timeout",
                };
                if reason != "runtime_cleanup_pending" {
                    tracing::warn!(reason_code = reason, "runtime reconciliation deferred");
                }
                let _ = reconciler.record_failure(&profile, reason).await;
            });
        }
        while tasks.join_next().await.is_some() {}
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one reconciliation pass binds source observation and frozen registry credentials"
    )]
    async fn profile(&self, id: &str) -> Result<(), &'static str> {
        let time = now()?;
        let Some(ProfileSelection {
            repository,
            mut cursor,
            registry,
            mut selected_revision,
            certificate,
        }) = self.select_profile(id).await?
        else {
            return Ok(());
        };
        if self.resume_accepted_cleanup(id, time, &mut cursor).await? {
            return Ok(());
        }
        self.resume_profile_cleanup(id, &mut cursor).await?;
        if cursor.next_at > time {
            return Ok(());
        }
        let mut runtime = self.preparation_runtime(id, &cursor, &registry).await?;
        let github = configuration::github_identity(repository.provider)?;
        let token = self
            .source
            .token(github.installation_id, &github.repository_id)
            .await?;
        let head = self
            .source
            .head(
                &github.owner,
                &github.name,
                &repository.default_branch,
                &token,
            )
            .await?;
        // A source change that produces a new lock never reuses the previous
        // mutable tag. The repository portion is deployment controlled; the
        // fingerprint is the immutable image identity used for resolve-or-build.
        let (policy, lock) = self.repository_inputs(&github, &head, &token).await?;
        let lock_identity = crate::profiles::ayni::environment::parse_lock(&lock)
            .map_err(|_| "repository_lock_invalid")?;
        ayni_orchestrator_profile_ayni::compatibility::validate_lock_envelope(
            &lock,
            &runtime.platform,
        )?;
        // A changed main must stop admitting tasks before resolving/building its
        // environment, even when that preparation takes several reconciliation passes.
        selected_revision = Some(
            self.observe_source_selection(id, &head, &lock_identity, selected_revision, &registry)
                .await?,
        );
        runtime = runtime.for_lock_fingerprint(&lock_identity.fingerprint)?;
        if !self
            .freeze_preparation(
                id,
                &runtime,
                &head,
                &policy,
                &lock,
                &lock_identity,
                &registry,
                certificate.as_ref(),
                &mut cursor,
            )
            .await?
        {
            return Ok(());
        }
        if cursor.retry_exhausted() {
            return Ok(());
        }
        if cursor.pending.is_none()
            && !self
                .resolve_builder(id, &lock_identity.ayni_version, &mut runtime, &mut cursor)
                .await?
        {
            return Ok(());
        }
        if !self
            .prepare_certificate_trust(
                id,
                &mut runtime,
                &github,
                &repository.default_branch,
                &head,
                &token,
                certificate.as_ref(),
                &mut cursor,
            )
            .await?
        {
            return Ok(());
        }
        let (runtime, source_digest) =
            normalized_runtime_source(&runtime, &lock_identity.fingerprint)?;
        let binding = cursor
            .pending
            .as_ref()
            .and_then(|pending| pending.certificate.as_ref())
            .or(certificate.as_ref());
        let source_digest = certificate_source_digest(&source_digest, binding)?;
        let observation = ProfileObservation {
            id,
            runtime: &runtime,
            time,
            github,
            head,
            token,
            source_digest,
            default_branch: repository.default_branch,
            selected_revision,
            registry,
            certificate,
        };
        if cursor.pending.is_some() {
            self.observe_pending(&observation, &mut cursor).await
        } else {
            self.begin_observation(&observation, &mut cursor).await
        }
    }

    async fn observe_source_selection(
        &self,
        id: &str,
        head: &str,
        lock_identity: &crate::profiles::ayni::environment::LockIdentity,
        previous_revision: Option<String>,
        registry: &v1::RuntimeRegistrySelection,
    ) -> Result<String, &'static str> {
        // Pending operations retain their old credentials/destination for cleanup;
        // the new source must bind the repository's current desired destination.
        let desired = self.configured_profile(id).await?;
        let repository_image =
            ayni_orchestrator_application::execution_settings::image_repository(&desired.image)
                .ok_or("runtime_image_invalid")?;
        let mut command = crate::profiles::ayni::runtime_selection::command(
            id,
            head,
            &lock_identity.fingerprint,
            repository_image,
            "",
            &lock_identity.ayni_version,
        )
        .map_err(|_| "runtime_revision_failed")?;
        // The resolved infrastructure identity is selected after preparation.
        command.identity.source_digest = None;
        let registry = registry.clone();
        self.with_database(move |store| {
            store
                .observe_runtime_source(&command, previous_revision.as_deref(), &registry)
                .map_err(|_| "runtime_source_changed")
        })
        .await
    }

    async fn resume_profile_cleanup(
        &self,
        id: &str,
        cursor: &mut RuntimeCursor,
    ) -> Result<(), &'static str> {
        if cursor.resume_cleanup() {
            self.save(id, cursor).await?;
        }
        if cursor.retry_exhausted()
            || cursor
                .pending
                .as_ref()
                .is_some_and(|pending| pending.failed)
        {
            // Terminal workload cleanup must progress even while source/provider observation is unavailable.
            self.cleanup_exhausted_cursor(cursor).await?;
        }
        Ok(())
    }

    async fn pending_verification_phase(
        &self,
        observation: &ProfileObservation<'_>,
        pending: &PendingVerification,
    ) -> Result<PendingPhase, &'static str> {
        let lookup_id = observation.id.to_owned();
        let pending_revision = pending.profile_revision.clone();
        let selected = self
            .with_database(move |store| {
                let profile = store
                    .runtime_profile(&lookup_id)
                    .map_err(|_| "runtime_profile_missing")?;
                Ok(profile.active_revision_id.as_deref() == Some(pending_revision.as_str()))
            })
            .await?;
        let phase = if !selected
            && (pending.build_name.is_none() || pending.phase.publication_observed())
        {
            PendingPhase::DiscardStale
        } else {
            pending_phase(
                SourceIdentity {
                    revision: &pending.request.revision,
                    configuration_digest: &pending.source_digest,
                },
                SourceIdentity {
                    revision: &observation.head,
                    configuration_digest: &observation.source_digest,
                },
                pending.build_name.is_some() && !pending.phase.publication_observed(),
            )
        };
        Ok(phase)
    }

    async fn begin_observation(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
    ) -> Result<(), &'static str> {
        let id = observation.id;
        let runtime = observation.runtime;
        let time = observation.time;
        let github = &observation.github;
        let head = observation.head.as_str();
        let token = observation.token.as_str();
        let source_digest = observation.source_digest.as_str();
        let bytes = self
            .source
            .lock(&github.owner, &github.name, head, token)
            .await?;
        let identity = crate::profiles::ayni::environment::parse_lock(&bytes)
            .map_err(|_| "repository_lock_invalid")?;
        let repository_image =
            ayni_orchestrator_application::execution_settings::image_repository(&runtime.image)
                .ok_or("runtime_image_invalid")?;
        let command = crate::profiles::ayni::runtime_selection::command(
            id,
            head,
            &identity.fingerprint,
            repository_image,
            source_digest,
            &identity.ayni_version,
        )
        .map_err(|_| "runtime_revision_failed")?;
        let expected_revision = observation.selected_revision.clone();
        let expected_registry = observation.registry.clone();
        let certificate = observation.certificate.clone();
        let next_cursor = cursor.clone();
        let id = id.to_owned();
        let runtime = runtime.clone();
        let github = github.clone();
        let head = head.to_owned();
        let source_digest = source_digest.to_owned();
        // Keep the guarded selection and durable Job intent in one serialized
        // database operation; UI writes cannot interleave between these writes.
        *cursor = self
            .with_database(move |store| {
                let revision = store
                    .select_observed_runtime_revision(
                        &command,
                        expected_revision.as_deref(),
                        &expected_registry,
                    )
                    .map_err(|_| "runtime_source_changed")?;
                let mut cursor = next_cursor;
                cursor.sequence = cursor
                    .sequence
                    .checked_add(1)
                    .ok_or("runtime_sequence_exhausted")?;
                cursor.pending = Some(PendingVerification {
                    observation: None,
                    name: operation_name(&id, cursor.sequence),
                    profile_revision: revision,
                    image: runtime.image.clone(),
                    frozen_runtime: Some(runtime.clone()),
                    started_at: time,
                    source_digest: source_digest.clone(),
                    lock_digest: format!("sha256:{:x}", Sha256::digest(&bytes)),
                    build_name: None,
                    phase: ImagePreparationPhase::default(),
                    candidate_image_id: None,
                    publish_name: None,
                    publish_attempts: 0,
                    publish_remaining: publication_budget(),
                    candidate_missing: false,
                    resolved_digest: None,
                    verification: None,
                    failed: false,
                    force_rebuild: cursor.force_rebuild,
                    certificate,
                    request: ExecutionRequest {
                        repository_url: format!(
                            "https://github.com/{}/{}.git",
                            github.owner, github.name
                        ),
                        revision: head.clone(),
                        expected: RuntimeExpectation {
                            lock_fingerprint: format!("sha256:{}", identity.fingerprint),
                            platform: runtime.platform.clone(),
                            ayni_version: identity.ayni_version.clone(),
                        },
                    },
                });
                cursor.update_status(&runtime, identity.fingerprint)?;
                cursor.force_rebuild = false;
                store
                    .save_runtime_cursor(&id, &cursor)
                    .map_err(|_| "runtime_store_failed")?;
                Ok(cursor)
            })
            .await?;
        Ok(())
    }

    async fn save(&self, profile: &str, cursor: &RuntimeCursor) -> Result<(), &'static str> {
        let profile = profile.to_owned();
        let cursor = cursor.clone();
        self.with_database(move |store| {
            store
                .save_runtime_operation_cursor(&profile, &cursor)
                .map_err(|_| "runtime_store_failed")
        })
        .await
    }
}

mod coding;
mod coding_events;
mod preparation;
#[cfg(test)]
mod preparation_tests;
mod trust;
mod worker_mapping;

fn now() -> Result<u64, &'static str> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| "clock_invalid")
}

impl RuntimeReconciler<RuntimeKubernetes> {
    pub fn observation_namespace(&self) -> &str {
        &self.configuration.namespace
    }

    /// Constructs execution adapters from the deployed process configuration.
    ///
    /// # Errors
    /// Returns a safe reason code for invalid configuration or unavailable adapters.
    pub fn from_environment(state: AppState) -> Result<Option<Arc<Self>>, &'static str> {
        let Some(configuration) = RuntimeConfiguration::from_environment()? else {
            return Ok(None);
        };
        let source = state.source.clone().ok_or("github_client_unavailable")?;
        let kubernetes = RuntimeKubernetes::new(configuration.execution_configuration())?;
        Ok(Some(Arc::new(Self {
            state,
            configuration,
            source,
            kubernetes,
            pass: Mutex::new(()),
            coding_pass: Mutex::new(()),
            credential_refresh: Mutex::new(BTreeMap::new()),
        })))
    }
}

fn normalized_runtime_source(
    runtime: &ProfileRuntime,
    fingerprint: &str,
) -> Result<(ProfileRuntime, String), &'static str> {
    let runtime = runtime.for_lock_fingerprint(fingerprint)?;
    let digest = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&runtime).map_err(|_| "runtime_source_invalid")?)
    );
    Ok((runtime, digest))
}

fn certificate_source_digest(
    source: &str,
    certificate: Option<&SigningIdentity>,
) -> Result<String, &'static str> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(
            serde_json::to_vec(&(source, certificate))
                .map_err(|_| "certificate_identity_invalid")?
        )
    ))
}

fn discovery_error(reason: &str) -> &'static str {
    match reason {
        "registry_access_denied" => "registry_access_denied",
        "registry_rate_limited" => "registry_rate_limited",
        "runtime_platform_mismatch" => "runtime_platform_mismatch",
        "registry_credentials_invalid" => "registry_credentials_invalid",
        "registry_provider_unsupported" => "registry_provider_unsupported",
        "runtime_digest_mismatch" => "runtime_digest_mismatch",
        _ => "registry_unavailable",
    }
}

#[cfg(test)]
mod source_identity_tests {
    use super::*;
    #[test]
    fn selecting_the_observed_lock_does_not_invalidate_pending_configuration()
    -> Result<(), &'static str> {
        let old = ProfileRuntime {
            image: format!("registry.test/team/runtime:lock-{}", "a".repeat(64)),
            platform: "linux/arm64".into(),
            service_account: "runtime-coding".into(),
            pull_secret: None,
            build: None,
        };
        let observed = "b".repeat(64);
        let (selected, initial) = normalized_runtime_source(&old, &observed)?;
        let (_, next) = normalized_runtime_source(&selected, &observed)?;
        assert_eq!(initial, next);
        let mut changed = selected;
        changed.service_account = "different-account".into();
        assert_ne!(initial, normalized_runtime_source(&changed, &observed)?.1);
        Ok(())
    }
}

mod configuration;
mod official_builder;
use configuration::{ProfileSelection, configured_managed_profile};

mod activation;
mod verification_events;
mod verification_tracing;

mod runtime_operations;
