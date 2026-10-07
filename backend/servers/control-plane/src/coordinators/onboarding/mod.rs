//! Existing-App repository onboarding. Provider credentials never enter the BFF.
use crate::{
    AppState, adapters::github::SourceClient, coordinators::with_store_async as with_store,
};
use ayni_orchestrator_api::{v1, validation};
use std::sync::Arc;
mod error;
mod setup;
use crate::profiles::ayni::onboarding_templates;
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
        match SourceClient::from_environment(20) {
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

    pub(crate) async fn connect_github(
        &self,
        request: &v1::ConnectGithubRepositoryRequest,
    ) -> Result<v1::RepositoryConnection, OnboardingError> {
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
        let observed = self
            .onboarding_source()?
            .discover_repository(&request.owner, &request.name)
            .await
            .map_err(provider_error)?;
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
        self.enrol_connected_repository(&connection).await;
        Ok(connection)
    }

    async fn enrol_connected_repository(&self, connection: &v1::RepositoryConnection) {
        let Ok(inspection) = self.inspect_ayni(&connection.id).await else {
            tracing::warn!(
                repository_connection_id = connection.id,
                "automatic repository inspection deferred"
            );
            return;
        };
        if !inspection.policy_present || !inspection.lock_compatible {
            tracing::info!(
                repository_connection_id = connection.id,
                reason = inspection.reason,
                "repository is not an Ayni-managed runtime candidate"
            );
            return;
        }
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
                            // Kept for wire compatibility; the producer is an internal
                            // Kubernetes Job, never a repository workflow.
                            workflow_template_version: "internal-build-v1".into(),
                            oci_repository: inspection.oci_repository,
                            runtime_source_digest: None,
                        },
                    ),
                ),
            }),
        };
        let _ = with_store(self, move |store| {
            if !store
                .list_runtime_profiles(Some(&request.repository_connection_id), None, None, None)?
                .items
                .is_empty()
            {
                return Ok(());
            }
            store
                .create_runtime_profile(&request.idempotency_key, &request)
                .map(|_| ())
        })
        .await
        .map_err(|error| {
            tracing::warn!(
                repository_connection_id = connection.id,
                ?error,
                "automatic runtime profile enrollment deferred"
            );
        });
    }

    pub(crate) async fn inspect_ayni(
        &self,
        repository_id: &str,
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
        let mut result = v1::InspectAyniRepositoryResponse {
            repository_snapshot_id: snapshot.id,
            repository_revision: head.clone(),
            ayni_version: ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION.into(),
            workflow_template_version: "internal-build-v1".into(),
            oci_repository: format!(
                "ghcr.io/{}-ayni",
                format!("{}/{}", github.owner, github.name).to_lowercase()
            ),
            workflows: vec![],
            ..Default::default()
        };
        let policy = source
            .file(&github.owner, &github.name, &head, ".ayni.toml", &token)
            .await;
        result.policy_present = match policy {
            Ok(bytes) => !bytes.is_empty(),
            Err("github_not_found") => false,
            Err(reason) => return Err(provider_error(reason)),
        };
        let lock = match source
            .lock(&github.owner, &github.name, &head, &token)
            .await
        {
            Ok(bytes) => bytes,
            Err("github_not_found") => {
                result.reason = "lock_missing".into();
                return Ok(result);
            }
            Err(reason) => return Err(provider_error(reason)),
        };
        let inspection =
            crate::profiles::ayni::inspection::inspect_lock(&lock, result.policy_present);
        result.lock_compatible = inspection.lock_compatible;
        result.lock_fingerprint = inspection.lock_fingerprint;
        result.trust_policy_pinned = inspection.trust_policy_pinned;
        result.reason = inspection.reason.into();
        Ok(result)
    }

    /// Repository connections are inspected at creation time and runtime
    /// reconciliation owns all later registry/build work. It never mutates a
    /// connected repository.
    pub async fn reconcile_onboarding(&self) {
        if let Err(reason) = self.reconcile_setup_actions().await {
            tracing::warn!(
                reason_code = reason,
                "repository setup reconciliation deferred"
            );
        }
    }
}
