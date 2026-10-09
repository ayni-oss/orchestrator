//! Existing-App repository onboarding. Provider credentials never enter the BFF.
use crate::profiles::ayni::preparation_state::RuntimeCursorExt;
use crate::{
    AppState, adapters::github::SourceClient, coordinators::with_store_async as with_store,
};
use ayni_orchestrator_api::{v1, validation};
use std::sync::Arc;
mod error;

pub use error::{FailureKind, OnboardingError};

pub fn provider_error(reason: &'static str) -> OnboardingError {
    tracing::warn!(
        provider = "github",
        reason_code = reason,
        "repository provider request failed"
    );
    let public_reason = match reason {
        "github_app_unconfigured" => "repository_provider_unconfigured",
        "github_access_denied" | "github_not_found" | "github_permissions_missing" => {
            "provider_access_required"
        }
        "github_rate_limited" | "github_unavailable" => "provider_temporarily_unavailable",
        _ => "repository_provider_failed",
    };
    OnboardingError::failed_precondition(public_reason)
}

fn github(repository: &v1::RepositoryConnection) -> Result<&v1::GithubRepository, OnboardingError> {
    match repository
        .provider
        .as_ref()
        .and_then(|p| p.provider.as_ref())
    {
        Some(v1::repository_provider::Provider::Github(value)) => Ok(value),
        _ => Err(OnboardingError::failed_precondition(
            "repository_provider_unsupported",
        )),
    }
}

impl AppState {
    /// Enables the existing GitHub App adapter when deployment credentials are available.
    #[must_use]
    pub fn with_github_onboarding(mut self) -> Self {
        match SourceClient::unconfigured(20) {
            Ok(source) => self.source = Some(Arc::new(source)),
            Err(reason) => {
                tracing::warn!(reason_code = reason, "repository onboarding unavailable");
            }
        }
        self
    }

    pub(crate) fn onboarding_source(&self) -> Result<&SourceClient, OnboardingError> {
        self.source.as_deref().ok_or_else(|| {
            let error = provider_error("github_app_unconfigured");
            OnboardingError::unavailable(error.reason())
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "discovery intent, provider identity and enrollment have one ordered persistence boundary"
    )]
    pub(crate) async fn connect_github(
        &self,
        request: &v1::ConnectGithubRepositoryRequest,
    ) -> Result<v1::RepositoryConnection, OnboardingError> {
        let _lifecycle = self.repository_lifecycle.read().await;
        let valid_name = |value: &str| {
            !value.is_empty()
                && value.len() <= 100
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        };
        if !valid_name(&request.owner) || !valid_name(&request.name) {
            return Err(OnboardingError::invalid_argument("repository_name_invalid"));
        }
        let intent = request.clone();
        let action = with_store(self, move |store| {
            store.begin_external_action(
                "ConnectGithubRepository",
                &intent.idempotency_key,
                &intent,
                "repository_discovery",
                &format!("{}/{}", intent.owner, intent.name),
            )
        })
        .await
        .map_err(|_| OnboardingError::failed_precondition("repository_connection_conflict"))?;
        if action.state == crate::adapters::sqlite::DurableActionState::Failed {
            return Err(OnboardingError::failed_precondition(
                "repository_discovery_failed_retry_with_new_key",
            ));
        }
        let observed = match async {
            self.onboarding_source()?
                .discover_repository(&request.owner, &request.name)
                .await
                .map_err(provider_error)
        }
        .await
        {
            Ok(observed) => observed,
            Err(error) => {
                let key = request.idempotency_key.clone();
                with_store(self, move |store| {
                    store.fail_external_action("ConnectGithubRepository", &key, error.reason())
                })
                .await
                .map_err(|_| {
                    OnboardingError::internal("repository_onboarding_persistence_failed")
                })?;
                return Err(error);
            }
        };
        let field = |key: &str| {
            observed[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| provider_error("github_repository_invalid"))
        };
        let owner = observed["owner"]["login"]
            .as_str()
            .ok_or_else(|| provider_error("github_repository_invalid"))?;
        let name = field("name")?;
        let message = v1::CreateRepositoryConnectionRequest {
            idempotency_key: request.idempotency_key.clone(),
            provider: Some(v1::RepositoryProvider {
                provider: Some(v1::repository_provider::Provider::Github(
                    v1::GithubRepository {
                        owner: owner.into(),
                        name: name.clone(),
                        repository_id: observed["id"]
                            .as_u64()
                            .ok_or_else(|| provider_error("github_repository_invalid"))?
                            .to_string(),
                        installation_id: observed["installation_id"]
                            .as_u64()
                            .ok_or_else(|| provider_error("github_installation_invalid"))?,
                        html_url: format!("https://github.com/{owner}/{name}"),
                    },
                )),
            }),
            display_name: format!("{owner}/{name}"),
            default_branch: field("default_branch")?,
            registry_connection_id: request.registry_connection_id.clone(),
            image_namespace: request.image_namespace.clone(),
        };
        validation::create_repository_connection(&message)
            .map_err(|_| OnboardingError::invalid_argument("repository_identity_invalid"))?;
        let connection = with_store(self, move |store| {
            let connection =
                store.create_repository_connection(&message.idempotency_key, &message)?;
            if connection.status == v1::RepositoryConnectionStatus::Connected as i32 {
                return Ok(connection);
            }
            store.transition_repository_connection(
                &connection.id,
                connection.resource_version,
                v1::RepositoryConnectionStatus::Connected,
                None,
            )
        })
        .await
        .map_err(|_| OnboardingError::failed_precondition("repository_connection_conflict"))?;

        // Connection is deliberately a read-only repository operation.  As soon as
        // it succeeds, freeze the default-branch observation and create the managed
        // profile for a supported committed Ayni configuration.  The runtime
        // reconciler owns resolve-or-build; no workflow or pull request is installed
        // in the connected repository.
        let result = self.analyze_repository_inner(&connection.id).await?;
        let key = request.idempotency_key.clone();
        with_store(self, move |store| {
            store.complete_external_action("ConnectGithubRepository", &key)
        })
        .await
        .map_err(|_| OnboardingError::internal("repository_onboarding_persistence_failed"))?;
        Ok(result)
    }

    /// Restart interrupted discovery and inspection using durable repository state.
    pub async fn reconcile_repository_onboarding(&self) {
        let Ok(discoveries) = with_store(self, |store| {
            store.pending_external_actions("ConnectGithubRepository")
        })
        .await
        else {
            return;
        };
        for action in discoveries {
            use prost::Message;
            if let Ok(request) =
                v1::ConnectGithubRepositoryRequest::decode(action.request_payload.as_slice())
            {
                let _ = self.connect_github(&request).await;
            }
        }
        let Ok(actions) = with_store(self, |store| {
            store.pending_external_actions("AnalyzeRepository")
        })
        .await
        else {
            return;
        };
        for action in actions {
            use prost::Message;
            let Ok(request) =
                v1::AnalyzeRepositoryRequest::decode(action.request_payload.as_slice())
            else {
                continue;
            };
            if self
                .analyze_repository(&request.repository_connection_id)
                .await
                .is_ok()
            {
                let _ = with_store(self, move |store| {
                    store.complete_external_action("AnalyzeRepository", &action.idempotency_key)
                })
                .await;
            }
        }
        let mut page = v1::PageRequest {
            page_size: 100,
            page_token: String::new(),
        };
        loop {
            let request = page.clone();
            let Ok(repositories) = with_store(self, move |store| {
                store.list_repository_connections(None, Some(&request))
            })
            .await
            else {
                return;
            };
            for repository in repositories.items {
                if repository.status == v1::RepositoryConnectionStatus::Connecting as i32
                    || repository.onboarding_status.is_empty()
                    || matches!(
                        repository.onboarding_status.as_str(),
                        "pending" | "analyzing"
                    )
                {
                    let _ = self.analyze_repository(&repository.id).await;
                }
            }
            page.page_token = repositories.next_page_token;
            if page.page_token.is_empty() {
                break;
            }
        }
    }

    pub(crate) async fn analyze_repository(
        &self,
        id: &str,
    ) -> Result<v1::RepositoryConnection, OnboardingError> {
        let _lifecycle = self.repository_lifecycle.read().await;
        self.analyze_repository_inner(id).await
    }

    async fn analyze_repository_inner(
        &self,
        id: &str,
    ) -> Result<v1::RepositoryConnection, OnboardingError> {
        let _guard = self.connection_lock.lock().await;
        let id = id.to_owned();
        let lookup = id.clone();
        let connection = with_store(self, move |store| {
            store.record_onboarding(&lookup, "analyzing", "", "")
        })
        .await
        .map_err(|_| OnboardingError::not_found("repository_not_found"))?;
        let outcome = self.enrol_connected_repository(&connection).await;
        if outcome.is_err()
            && connection.status == v1::RepositoryConnectionStatus::Connecting as i32
        {
            let lookup = id.clone();
            with_store(self, move |store| {
                let current = store.repository_connection(&lookup)?;
                if current.status == v1::RepositoryConnectionStatus::Connecting as i32 {
                    store.transition_repository_connection(
                        &lookup,
                        current.resource_version,
                        v1::RepositoryConnectionStatus::Failed,
                        None,
                    )?;
                }
                Ok(())
            })
            .await
            .map_err(|_| OnboardingError::internal("repository_onboarding_persistence_failed"))?;
        }
        let (status, reason, revision) = match outcome {
            Ok(revision) => ("enrolled", "", revision),
            Err(error) => (
                "failed",
                error.reason(),
                connection.onboarding_revision.clone(),
            ),
        };
        with_store(self, move |store| {
            let revision = if revision.is_empty() {
                store.repository_connection(&id)?.onboarding_revision
            } else {
                revision
            };
            store.record_onboarding(&id, status, reason, &revision)
        })
        .await
        .map_err(|_| OnboardingError::internal("repository_onboarding_persistence_failed"))
    }

    async fn enrol_connected_repository(
        &self,
        connection: &v1::RepositoryConnection,
    ) -> Result<String, OnboardingError> {
        let github = github(connection)?;
        if connection.status != v1::RepositoryConnectionStatus::Connected as i32 {
            let observed = self
                .onboarding_source()?
                .discover_repository(&github.owner, &github.name)
                .await
                .map_err(provider_error)?;
            if observed["id"]
                .as_u64()
                .map(|value| value.to_string())
                .as_deref()
                != Some(github.repository_id.as_str())
            {
                return Err(OnboardingError::failed_precondition(
                    "repository_identity_changed",
                ));
            }
            let id = connection.id.clone();
            let version = connection.resource_version;
            with_store(self, move |store| {
                store.transition_repository_connection(
                    &id,
                    version,
                    v1::RepositoryConnectionStatus::Connected,
                    None,
                )
            })
            .await
            .map_err(|_| OnboardingError::failed_precondition("repository_connection_conflict"))?;
        }
        let inspection = self.inspect_ayni_snapshot(&connection.id, true).await?;
        if !inspection.policy_present || !inspection.lock_compatible {
            return Err(OnboardingError::failed_precondition(
                match inspection.reason.as_str() {
                    "policy_missing" => "policy_missing",
                    "lock_missing" => "lock_missing",
                    "runtime_version_unsupported" => "runtime_version_unsupported",
                    "lock_invalid" => "repository_lock_invalid",
                    _ => "runtime_lock_contract_unsupported",
                },
            ));
        }
        let revision = inspection.repository_revision.clone();
        let request = v1::CreateRuntimeProfileRequest {
            idempotency_key: format!("connect:{}:{}", connection.id, inspection.lock_fingerprint),
            repository_connection_id: connection.id.clone(),
            repository_snapshot_id: inspection.repository_snapshot_id,
            configuration: Some(v1::RuntimeProfileConfiguration {
                configuration: Some(
                    v1::runtime_profile_configuration::Configuration::AyniManaged(
                        v1::AyniManagedProfileRevision {
                            lock_fingerprint: inspection.lock_fingerprint,
                            ayni_version: inspection.ayni_version,
                            oci_repository: inspection.oci_repository,
                            runtime_source_digest: None,
                        },
                    ),
                ),
            }),
        };
        with_store(self, move |store| {
            let profiles = store
                .list_runtime_profiles(Some(&request.repository_connection_id), None, None, None)?
                .items;
            if !profiles.is_empty() {
                for profile in profiles {
                    let mut cursor = store.runtime_cursor(&profile.id)?;
                    cursor.retry();
                    store.save_runtime_cursor(&profile.id, &cursor)?;
                }
                return Ok(());
            }
            store
                .create_runtime_profile(&request.idempotency_key, &request)
                .map(|_| ())
        })
        .await
        .map_err(|_| OnboardingError::internal("runtime_profile_enrollment_failed"))?;
        Ok(revision)
    }

    pub(crate) async fn inspect_ayni(
        &self,
        repository_id: &str,
    ) -> Result<v1::InspectAyniRepositoryResponse, OnboardingError> {
        self.inspect_ayni_snapshot(repository_id, false).await
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one immutable source observation produces a bounded inspection response"
    )]
    async fn inspect_ayni_snapshot(
        &self,
        repository_id: &str,
        persist_observation: bool,
    ) -> Result<v1::InspectAyniRepositoryResponse, OnboardingError> {
        let lookup_id = repository_id.to_owned();
        let repository = with_store(self, move |s| s.repository_connection(&lookup_id))
            .await
            .map_err(|_| OnboardingError::not_found("repository_not_found"))?;
        let github = github(&repository)?;
        let source = self.onboarding_source()?;
        let token = source
            .token(github.installation_id, &github.repository_id)
            .await
            .map_err(provider_error)?;
        let head = source
            .head(
                &github.owner,
                &github.name,
                &repository.default_branch,
                &token,
            )
            .await
            .map_err(provider_error)?;
        let snapshot_repository = repository_id.to_owned();
        let snapshot_head = head.clone();
        let snapshot = with_store(self, move |s| {
            s.observe_repository_snapshot(&snapshot_repository, &snapshot_head)
        })
        .await
        .map_err(|_| OnboardingError::internal("repository_snapshot_failed"))?;
        if persist_observation {
            let observed_id = repository_id.to_owned();
            let observed_revision = head.clone();
            with_store(self, move |store| {
                store.record_onboarding(&observed_id, "analyzing", "", &observed_revision)
            })
            .await
            .map_err(|_| OnboardingError::internal("repository_onboarding_persistence_failed"))?;
        }
        let mut result = v1::InspectAyniRepositoryResponse {
            repository_snapshot_id: snapshot.id,
            repository_revision: head.clone(),
            oci_repository: format!(
                "ghcr.io/{}/{}",
                github.owner.to_ascii_lowercase(),
                github.name.to_ascii_lowercase()
            ),
            ..Default::default()
        };
        let policy = source
            .file(&github.owner, &github.name, &head, ".ayni.toml", &token)
            .await;
        result.policy_present = match policy {
            Ok(bytes) => {
                if bytes.is_empty()
                    || bytes.len() > 1_048_576
                    || std::str::from_utf8(&bytes)
                        .ok()
                        .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
                        .is_none()
                {
                    return Err(OnboardingError::failed_precondition(
                        "repository_policy_invalid",
                    ));
                }
                true
            }
            Err("github_not_found") => false,
            Err(reason) => return Err(provider_error(reason)),
        };
        if !result.policy_present {
            result
                .findings
                .push(crate::profiles::ayni::findings::finding("policy_missing"));
        }
        let lock = match source
            .lock(&github.owner, &github.name, &head, &token)
            .await
        {
            Ok(bytes) => bytes,
            Err("github_not_found") => {
                result.reason = "lock_missing".into();
                result
                    .findings
                    .push(crate::profiles::ayni::findings::finding("lock_missing"));
                return Ok(result);
            }
            Err(reason) => return Err(provider_error(reason)),
        };
        let inspection =
            crate::profiles::ayni::inspection::inspect_lock(&lock, result.policy_present);
        result.ayni_version = inspection.ayni_version;
        result.lock_compatible = inspection.lock_compatible;
        result.lock_fingerprint = inspection.lock_fingerprint;
        result.trust_policy_pinned = inspection.trust_policy_pinned;
        result.reason = inspection.reason.into();
        if !result.lock_compatible {
            result
                .findings
                .push(crate::profiles::ayni::findings::finding(
                    "lock_incompatible",
                ));
        }
        if !result.trust_policy_pinned {
            result
                .findings
                .push(crate::profiles::ayni::findings::finding(
                    "trust_policy_missing",
                ));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::DurableActionState;

    #[tokio::test]
    async fn failed_discovery_is_durable_without_inventing_repository_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let state = AppState::open(directory.path().join("state.sqlite"), 3)?;
        let request = v1::ConnectGithubRepositoryRequest {
            idempotency_key: "discover-one".into(),
            owner: "example".into(),
            name: "repository".into(),
            ..Default::default()
        };
        assert!(state.connect_github(&request).await.is_err());
        with_store(&state, move |store| {
            let action = store.begin_external_action(
                "ConnectGithubRepository",
                "discover-one",
                &request,
                "repository_discovery",
                "example/repository",
            )?;
            assert_eq!(action.state, DurableActionState::Failed);
            assert!(
                store
                    .list_repository_connections(None, None)?
                    .items
                    .is_empty()
            );
            store.integrity_check()
        })
        .await?;
        state.shutdown_database().await?;
        let reopened = AppState::open(directory.path().join("state.sqlite"), 3)?;
        reopened.shutdown_database().await?;
        Ok(())
    }

    #[tokio::test]
    async fn stranded_connection_terminates_visibly_when_discovery_is_unavailable()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let state = AppState::open(directory.path().join("state.sqlite"), 3)?;
        let connection = with_store(&state, |store| {
            store.create_repository_connection(
                "connecting",
                &v1::CreateRepositoryConnectionRequest {
                    idempotency_key: "connecting".into(),
                    display_name: "example/repository".into(),
                    default_branch: "main".into(),
                    provider: Some(v1::RepositoryProvider {
                        provider: Some(v1::repository_provider::Provider::Github(
                            v1::GithubRepository {
                                owner: "example".into(),
                                name: "repository".into(),
                                repository_id: "123".into(),
                                installation_id: 1,
                                html_url: "https://github.com/example/repository".into(),
                            },
                        )),
                    }),
                    ..Default::default()
                },
            )
        })
        .await?;
        state.reconcile_repository_onboarding().await;
        let id = connection.id;
        let failed = with_store(&state, move |store| store.repository_connection(&id)).await?;
        assert_eq!(failed.status, v1::RepositoryConnectionStatus::Failed as i32);
        assert_eq!(failed.onboarding_status, "failed");
        assert_eq!(failed.onboarding_reason, "repository_provider_unconfigured");
        state.reconcile_repository_onboarding().await;
        let id = failed.id.clone();
        assert_eq!(
            with_store(&state, move |store| store.repository_connection(&id)).await?,
            failed
        );
        state.shutdown_database().await?;
        Ok(())
    }
}
