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
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::Mutex, task::JoinSet};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct RuntimeCursor {
    sequence: u64,
    next_at: u64,
    last_success: Option<u64>,
    failures: u32,
    pending: Option<PendingVerification>,
    bindings: BTreeMap<String, String>,
    status: BTreeMap<String, String>,
    #[serde(default)]
    force_rebuild: bool,
    #[serde(default)]
    trust: Option<TrustUpdate>,
}

impl RuntimeCursor {
    fn request_rebuild(&mut self, requested: bool) {
        self.force_rebuild |= requested || self.trust.as_ref().is_some_and(TrustUpdate::failed);
    }

    fn update_status(
        &mut self,
        runtime: &ProfileRuntime,
        fingerprint: String,
    ) -> Result<(), &'static str> {
        self.status.insert(
            "source-kind".into(),
            if runtime.build.is_some() {
                "build"
            } else {
                "registry"
            }
            .into(),
        );
        self.status
            .insert("discovery-reference".into(), runtime.image.clone());
        self.status.insert(
            "default-sha".into(),
            self.pending
                .as_ref()
                .ok_or("runtime_self_invalid")?
                .request
                .revision
                .clone(),
        );
        self.status.insert("lock-fingerprint".into(), fingerprint);
        if let Some(name) = self.pending.as_ref().and_then(|p| p.build_name.clone()) {
            self.status.insert("build-job".into(), name);
        } else {
            self.status.remove("build-job");
        }
        Ok(())
    }

    pub(crate) fn metadata(&self) -> Vec<v1::VerifiedMetadata> {
        let mut status = self.status.clone();
        status.insert("next-retry".into(), self.next_at.to_string());
        if let Some(last) = self.last_success {
            status.insert("last-success".into(), last.to_string());
        }
        status
            .into_iter()
            .map(|(key, value)| v1::VerifiedMetadata { key, value })
            .collect()
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct PendingVerification {
    name: String,
    profile_revision: String,
    request: ExecutionRequest,
    image: String,
    #[serde(default)]
    frozen_runtime: Option<ProfileRuntime>,
    started_at: u64,
    source_digest: String,
    lock_digest: String,
    build_name: Option<String>,
    #[serde(flatten)]
    build: BuildProgress,
    #[serde(default)]
    candidate_image_id: Option<String>,
    #[serde(default)]
    publish_name: Option<String>,
    #[serde(default)]
    publish_attempts: u32,
    #[serde(default = "publication_budget")]
    publish_remaining: u8,
    #[serde(default)]
    candidate_missing: bool,
    #[serde(default)]
    force_rebuild: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    certificate: Option<SigningIdentity>,
    #[serde(flatten)]
    discovery: DiscoveryProgress,
    #[serde(default)]
    resolved_digest: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    failed: bool,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct BuildProgress {
    #[serde(default, rename = "build_started")]
    started: bool,
    #[serde(default, rename = "build_completed")]
    completed: bool,
    #[serde(default, rename = "build_observed")]
    observed: bool,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct DiscoveryProgress {
    #[serde(default, rename = "discovery_started")]
    started: bool,
    #[serde(default, rename = "discovery_complete")]
    complete: bool,
}

const fn publication_budget() -> u8 {
    3
}

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
        let mut configuration = self.configuration.clone();
        self.with_database(move |store| {
            let settings = store
                .build_settings()
                .map_err(|_| "build_settings_unavailable")?;
            if let Some(managed) = &mut configuration.managed_runtime {
                managed.build = if settings.security_mode == configuration.engine_mode
                    && settings.status == "ready"
                    && settings.tested_revision == settings.resource_version
                {
                    managed.platform.clone_from(&settings.platform);
                    Some(crate::coordinators::build_service::runtime(
                        &settings,
                        "runtime-push".into(),
                        String::new(),
                    ))
                } else {
                    None
                };
            }
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
                tracing::warn!(reason_code = reason, "runtime reconciliation deferred");
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
        let lookup_id = id.to_owned();
        let Some((repository, mut cursor, registry, selected_revision, certificate)) = self
            .with_database(move |store| {
                store
                    .invalidate_certificate_runtime_profiles()
                    .map_err(|_| "certificate_selection_invalid")?;
                let profile = store
                    .normalize_runtime_destination(&lookup_id)
                    .map_err(|_| "runtime_profile_missing")?;
                if profile.kind != v1::RuntimeProfileKind::AyniManaged as i32 {
                    return Ok(None);
                }
                let repository = store
                    .repository_connection(&profile.repository_connection_id)
                    .map_err(|_| "repository_missing")?;
                let mut cursor = store
                    .runtime_cursor(&lookup_id)
                    .map_err(|_| "runtime_cursor_invalid")?;
                let actions = store
                    .pending_external_actions("ReconcileRuntimeProfile")
                    .map_err(|_| "runtime_action_invalid")?;
                for action in actions {
                    use prost::Message;
                    let request = v1::ReconcileRuntimeProfileRequest::decode(
                        action.request_payload.as_slice(),
                    )
                    .map_err(|_| "runtime_action_invalid")?;
                    if request.runtime_profile_id != lookup_id {
                        continue;
                    }
                    cursor.next_at = 0;
                    cursor.failures = 0;
                    cursor.request_rebuild(request.force_rebuild);
                    // Explicit retry releases a failed verification for a fresh observation.
                    if let Some(pending) = &mut cursor.pending {
                        pending.failed = false;
                        if !request.force_rebuild
                            && cursor
                                .status
                                .get("reason-code")
                                .is_some_and(|reason| reason == "runtime_publication_failed")
                        {
                            pending.publish_remaining = publication_budget();
                            pending.discovery.complete = false;
                            pending.discovery.started = false;
                            pending.started_at = now().map_err(|_| "runtime_clock_invalid")?;
                        } else if request.force_rebuild || cursor.status.contains_key("reason-code")
                        {
                            pending.source_digest.clear();
                        }
                    }
                    cursor.status.remove("reason-code");
                    // Persist the wake-up before acknowledging the action. Replaying
                    // after a crash is harmless and never resets build intent.
                    store
                        .save_runtime_cursor(&lookup_id, &cursor)
                        .map_err(|_| "runtime_cursor_invalid")?;
                    store
                        .complete_external_action(
                            "ReconcileRuntimeProfile",
                            &action.idempotency_key,
                        )
                        .map_err(|_| "runtime_action_invalid")?;
                }
                let registry = store
                    .runtime_registry(&profile)
                    .map_err(|_| "runtime_registry_invalid")?;
                let certificate = store
                    .effective_runtime_certificate(&profile)
                    .map_err(|_| "certificate_selection_invalid")?
                    .map(SigningIdentity::from);
                Ok(Some((
                    repository,
                    cursor,
                    registry,
                    profile.active_revision_id,
                    certificate,
                )))
            })
            .await?
        else {
            return Ok(());
        };
        if cursor.failures >= 3 {
            return self.cleanup_exhausted_cursor(&cursor).await;
        }
        if cursor.next_at > time {
            return Ok(());
        }
        if cursor.pending.is_none() && !registry.registry_connection_id.is_empty() {
            self.registry_ready(&registry.registry_connection_id)
                .await?;
        }
        let mut runtime = if let Some(frozen) = cursor
            .pending
            .as_ref()
            .and_then(|p| p.frozen_runtime.clone())
        {
            frozen
        } else {
            self.configured_profile(id).await?
        };
        let Some(v1::repository_provider::Provider::Github(github)) =
            repository.provider.and_then(|p| p.provider)
        else {
            return Err("repository_provider_unsupported");
        };
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
        if cursor.pending.is_none() {
            let certificate = certificate.as_ref().ok_or("certificate_setup_required")?;
            if !certificate.enabled || certificate.status != "ready" {
                return Err("certificate_not_ready");
            }
            if let Some(build) = &mut runtime.build {
                build.signing_secret.clone_from(&certificate.secret_name);
            }
            if !self
                .ensure_certificate_trust(
                    id,
                    &runtime,
                    &github,
                    &repository.default_branch,
                    &head,
                    &token,
                    certificate,
                    &mut cursor,
                )
                .await?
            {
                let lookup_id = id.to_owned();
                self.with_database(move |store| {
                    store
                        .runtime_failure(&lookup_id, "trust_policy_missing")
                        .map_err(|_| "runtime_cursor_invalid")
                })
                .await?;
                cursor
                    .status
                    .insert("reason-code".into(), "trust_policy_missing".into());
                cursor
                    .status
                    .insert("phase".into(), "setup_required".into());
                self.save(id, &cursor).await?;
                return Ok(());
            }
            cursor.status.remove("reason-code");
            cursor.status.insert("phase".into(), "analyzing".into());
            self.save(id, &cursor).await?;
        }
        let policy = self
            .source
            .file(&github.owner, &github.name, &head, ".ayni.toml", &token)
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
            .lock(&github.owner, &github.name, &head, &token)
            .await
            .map_err(|reason| {
                if reason == "github_not_found" {
                    "lock_missing"
                } else {
                    reason
                }
            })?;
        let lock_identity = crate::profiles::ayni::environment::parse_lock(&lock)
            .map_err(|_| "repository_lock_invalid")?;
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
        let phase = if !selected && (pending.build_name.is_none() || pending.build.observed) {
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
                pending.build_name.is_some() && !pending.build.observed,
            )
        };
        Ok(phase)
    }

    async fn accept(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
        digest: String,
        report: &ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport,
    ) -> Result<(), &'static str> {
        let id = observation.id;
        let runtime = observation.runtime;
        let time = observation.time;
        let github = &observation.github;
        let head = observation.head.as_str();
        let token = observation.token.as_str();
        let source_digest = observation.source_digest.as_str();
        let pending = cursor.pending.clone().ok_or("runtime_cursor_invalid")?;
        if let Some(certificate) = &pending.certificate {
            if report.runtime.certificate_key_id != certificate.key_id {
                return Err("certificate_verification_mismatch");
            }
        } else {
            return Err("certificate_setup_required");
        }
        crate::profiles::ayni::evidence_policy::admit_runtime(
            report,
            &pending.request.expected,
            head,
            &pending.lock_digest,
        )
        .map_err(crate::profiles::ayni::evidence_policy::EvidenceError::code)?;
        if self
            .source
            .head(
                &github.owner,
                &github.name,
                &observation.default_branch,
                token,
            )
            .await?
            != head
        {
            return Err("runtime_source_changed");
        }
        if pending.resolved_digest.as_deref() != Some(digest.as_str()) {
            return Err("runtime_digest_mismatch");
        }
        if let Some((_, expected)) = runtime.image.split_once('@')
            && expected != digest
        {
            return Err("runtime_digest_mismatch");
        }
        let key = format!(
            "{:x}",
            Sha256::digest(format!(
                "{}:{}:{}",
                source_digest, runtime.platform, report.runtime.lock_fingerprint
            ))
        );
        if !pending.force_rebuild
            && cursor
                .bindings
                .get(&key)
                .is_some_and(|bound| bound != &digest)
        {
            return Err("runtime_tag_drift");
        }
        if cursor.bindings.len() >= 128 && !cursor.bindings.contains_key(&key) {
            return Err("runtime_binding_limit");
        }
        cursor.bindings.insert(key, digest.clone());
        cursor
            .status
            .insert("resolved-digest".into(), digest.clone());
        // Retain the immutable binding before activation; retries can only replay it.
        self.save(id, cursor).await?;
        let command = crate::profiles::ayni::runtime_activation::command(
            id,
            &pending.profile_revision,
            &runtime.image,
            &digest,
            report,
        )
        .map_err(|_| "runtime_activation_rejected")?;
        self.with_database(move |store| {
            ayni_orchestrator_application::runtime_activation::activate(store, &command)
                .map_err(crate::adapters::sqlite::runtime_activation::activation_error)
                .map_err(|_| "runtime_activation_rejected")
        })
        .await?;
        self.kubernetes.cleanup(&pending.name).await?;
        if let Some(name) = &pending.build_name {
            self.kubernetes.cleanup(name).await?;
        }
        if let Some(name) = &pending.publish_name {
            self.kubernetes.cleanup(name).await?;
        }
        cursor.pending = None;
        cursor.last_success = Some(time);
        cursor.failures = 0;
        cursor.status.remove("reason-code");
        cursor.status.insert("phase".into(), "ready".into());
        let jitter =
            u64::from(Sha256::digest(id)[0]) % (self.configuration.interval_seconds / 5 + 1);
        cursor.next_at = time + self.configuration.interval_seconds + jitter;
        self.save(id, cursor).await?;
        Ok(())
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
                    name: operation_name(&id, cursor.sequence),
                    profile_revision: revision,
                    image: runtime.image.clone(),
                    frozen_runtime: Some(runtime.clone()),
                    started_at: time,
                    source_digest: source_digest.clone(),
                    lock_digest: format!("sha256:{:x}", Sha256::digest(&bytes)),
                    build_name: None,
                    build: BuildProgress::default(),
                    candidate_image_id: None,
                    publish_name: None,
                    publish_attempts: 0,
                    publish_remaining: publication_budget(),
                    candidate_missing: false,
                    discovery: DiscoveryProgress::default(),
                    resolved_digest: None,
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
                            ayni_version: ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION
                                .into(),
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
                .save_runtime_cursor(&profile, &cursor)
                .map_err(|_| "runtime_store_failed")
        })
        .await
    }
}

mod preparation;
#[cfg(test)]
mod preparation_tests;
mod trust;
use trust::{SigningIdentity, TrustUpdate};
mod coding;
mod worker_mapping;

fn now() -> Result<u64, &'static str> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| "clock_invalid")
}

impl RuntimeReconciler<RuntimeKubernetes> {
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

// Selecting a revision changes its stored lock; bind the observed lock before
// hashing configuration so that bookkeeping cannot invalidate pending work.
fn configured_managed_profile(
    store: &crate::adapters::sqlite::CleanStore,
    configuration: &RuntimeConfiguration,
    profile: &v1::RuntimeProfile,
    explicitly_configured: bool,
) -> Result<Option<ProfileRuntime>, &'static str> {
    let repository = store
        .repository_connection(&profile.repository_connection_id)
        .map_err(|_| "repository_missing")?;
    let registry = store
        .runtime_registry(profile)
        .map_err(|_| "runtime_registry_invalid")?;
    if explicitly_configured && registry.registry_connection_id.is_empty() {
        return Ok(None);
    }
    if repository.status != v1::RepositoryConnectionStatus::Connected as i32 {
        return Ok(None);
    }
    let Some(v1::repository_provider::Provider::Github(github)) = repository
        .provider
        .as_ref()
        .and_then(|provider| provider.provider.as_ref())
    else {
        return Ok(None);
    };
    let Some(revision_id) = &profile.active_revision_id else {
        return Ok(None);
    };
    let revision = store
        .runtime_profile_revision(revision_id)
        .map_err(|_| "runtime_revision_missing")?;
    let Some(v1::runtime_profile_revision::Configuration::AyniManaged(managed)) =
        revision.configuration
    else {
        return Ok(None);
    };
    let Some(runtime) = configuration.managed_runtime(
        &github.owner,
        &github.name,
        &managed.lock_fingerprint,
        &managed.oci_repository,
    ) else {
        return Ok(None);
    };
    if registry.registry_connection_id.is_empty() {
        return Ok(Some(runtime));
    }
    selected_registry_runtime(store, configuration, &registry, github, &managed)
}

fn selected_registry_runtime(
    store: &crate::adapters::sqlite::CleanStore,
    configuration: &RuntimeConfiguration,
    registry: &v1::RuntimeRegistrySelection,
    github: &v1::GithubRepository,
    managed: &v1::AyniManagedProfileRevision,
) -> Result<Option<ProfileRuntime>, &'static str> {
    let connection = store
        .get_connection(&registry.registry_connection_id)
        .map_err(|_| "registry_connection_missing")?;
    if connection.kind != "ghcr" {
        return Ok(None);
    }
    let mut managed_runtime = configuration
        .managed_runtime
        .clone()
        .ok_or("runtime_source_missing")?;
    managed_runtime.registry_prefix = format!("ghcr.io/{}", connection.image_namespace);
    managed_runtime.pull_secret =
        (!connection.pull_secret_name.is_empty()).then_some(connection.pull_secret_name);
    if let Some(build) = &mut managed_runtime.build {
        build.registry_secret = connection.secret_name;
    }
    let destination = crate::process::runtime_configuration::connection_image_repository(
        &connection.image_namespace,
        &github.owner,
        &github.name,
    )?;
    let mut runtime = managed_runtime.profile_runtime(
        &github.owner,
        &github.name,
        if managed.lock_fingerprint.is_empty() {
            "0000000000000000000000000000000000000000000000000000000000000000"
        } else {
            &managed.lock_fingerprint
        },
    )?;
    runtime.image = crate::process::runtime_configuration::managed_image_reference(
        &destination,
        &runtime.platform,
        if managed.lock_fingerprint.is_empty() {
            "0000000000000000000000000000000000000000000000000000000000000000"
        } else {
            &managed.lock_fingerprint
        },
    )?;
    Ok(Some(runtime))
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
