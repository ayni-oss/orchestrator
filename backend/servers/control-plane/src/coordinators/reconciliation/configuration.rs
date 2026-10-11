//! Bind operator execution infrastructure to repository-owned runtime destinations.
use super::{
    ExecutionBackend, ProfileRuntime, RuntimeConfiguration, RuntimeCursor, RuntimeReconciler,
    SigningIdentity, now, publication_budget, v1,
};
use crate::profiles::ayni::preparation_state::RuntimeCursorExt;
use ayni_orchestrator_application::runtime_preparation::ImagePreparationPhase;

// Selecting a revision changes its stored lock; bind the observed lock before
// hashing configuration so that bookkeeping cannot invalidate pending work.
pub(super) fn configured_managed_profile(
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
    if repository.use_ayni == Some(false)
        || repository.status != v1::RepositoryConnectionStatus::Connected as i32
    {
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

pub(super) struct ProfileSelection {
    pub repository: v1::RepositoryConnection,
    pub cursor: RuntimeCursor,
    pub registry: v1::RuntimeRegistrySelection,
    pub selected_revision: Option<String>,
    pub certificate: Option<SigningIdentity>,
}

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn select_profile(
        &self,
        id: &str,
    ) -> Result<Option<ProfileSelection>, &'static str> {
        let lookup_id = id.to_owned();
        self.with_database(move |store| {
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
                let request =
                    v1::ReconcileRuntimeProfileRequest::decode(action.request_payload.as_slice())
                        .map_err(|_| "runtime_action_invalid")?;
                if request.runtime_profile_id != lookup_id {
                    continue;
                }
                if !admit_retry(&mut cursor, &action.idempotency_key, request.force_rebuild)? {
                    store
                        .complete_external_action(
                            "ReconcileRuntimeProfile",
                            &action.idempotency_key,
                        )
                        .map_err(|_| "runtime_action_invalid")?;
                    continue;
                }
                // The retained admission fences replay after a crash.
                store
                    .save_runtime_cursor(&lookup_id, &cursor)
                    .map_err(|_| "runtime_cursor_invalid")?;
                store
                    .complete_external_action("ReconcileRuntimeProfile", &action.idempotency_key)
                    .map_err(|_| "runtime_action_invalid")?;
            }
            let registry = store
                .runtime_registry(&profile)
                .map_err(|_| "runtime_registry_invalid")?;
            let certificate = store
                .effective_runtime_certificate(&profile)
                .map_err(|_| "certificate_selection_invalid")?
                .map(SigningIdentity::from);
            Ok(Some(ProfileSelection {
                repository,
                cursor,
                registry,
                selected_revision: profile.active_revision_id,
                certificate,
            }))
        })
        .await
    }
}

/// Resolve the supported provider identity before any source token is requested.
pub(super) fn github_identity(
    provider: Option<v1::RepositoryProvider>,
) -> Result<v1::GithubRepository, &'static str> {
    match provider.and_then(|provider| provider.provider) {
        Some(v1::repository_provider::Provider::Github(github)) => Ok(github),
        _ => Err("repository_provider_unsupported"),
    }
}

/// Retain admission identity before acknowledgement, fencing crash replay of a used budget.
fn admit_retry(
    cursor: &mut RuntimeCursor,
    key: &str,
    force_rebuild: bool,
) -> Result<bool, &'static str> {
    if !cursor.accounting.admissions.insert(key.into()) {
        return Ok(false);
    }
    let failed_execution = cursor.failures >= 3
        || cursor
            .pending
            .as_ref()
            .is_some_and(|pending| pending.failed);
    let prior_reason = cursor.status.get("reason-code").cloned();
    let reporting_reexecution = cursor
        .pending
        .as_ref()
        .is_some_and(|pending| pending.verification.is_none())
        && prior_reason.as_deref().is_some_and(|reason| {
            matches!(
                reason,
                "runtime_report_missing"
                    | "runtime_reporting_failed"
                    | "runtime_report_identity_invalid"
                    | "runtime_report_oversized"
                    | "runtime_result_invalid"
                    | "runtime_pod_identity_invalid"
            )
        });
    cursor.retry();
    cursor.request_rebuild(force_rebuild);
    if failed_execution {
        cursor.force_rebuild |= cursor
            .pending
            .as_ref()
            .is_some_and(|pending| pending.force_rebuild);
    }
    if let Some(pending) = &mut cursor.pending {
        if !force_rebuild && prior_reason.as_deref() == Some("runtime_publication_failed") {
            pending.publish_remaining = publication_budget();
            pending.phase = ImagePreparationPhase::PublicationReadback;
            pending.started_at = now().map_err(|_| "runtime_clock_invalid")?;
        } else if force_rebuild || failed_execution || reporting_reexecution {
            pending.source_digest.clear();
        }
    }
    Ok(true)
}
