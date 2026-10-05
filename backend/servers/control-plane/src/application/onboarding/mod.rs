//! Existing-App repository onboarding. Provider credentials never enter the BFF.
use crate::{
    AppState, adapters::github::SourceClient, application::with_store_async as with_store,
    profiles::ayni::onboarding_templates,
};
use ayni_orchestrator_api::{v1, validation};
use serde_json::Value;
use std::sync::Arc;
use tonic::Status;

pub fn provider_error(reason: &'static str) -> Status {
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
    Status::failed_precondition(public_reason)
}

fn github(repository: &v1::RepositoryConnection) -> Result<&v1::GithubRepository, Status> {
    match repository
        .provider
        .as_ref()
        .and_then(|p| p.provider.as_ref())
    {
        Some(v1::repository_provider::Provider::Github(value)) => Ok(value),
        _ => Err(Status::failed_precondition(
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

    pub(crate) fn onboarding_source(&self) -> Result<&SourceClient, Status> {
        self.source.as_deref().ok_or_else(|| {
            let status = provider_error("github_app_unconfigured");
            Status::unavailable(status.message().to_owned())
        })
    }

    pub(crate) async fn connect_github(
        &self,
        request: &v1::ConnectGithubRepositoryRequest,
    ) -> Result<v1::RepositoryConnection, Status> {
        let valid_name = |value: &str| {
            !value.is_empty()
                && value.len() <= 100
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        };
        if !valid_name(&request.owner) || !valid_name(&request.name) {
            return Err(Status::invalid_argument("repository_name_invalid"));
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
            .map_err(|_| Status::invalid_argument("repository_identity_invalid"))?;
        with_store(self, move |store| {
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
        .map_err(|_| Status::failed_precondition("repository_connection_conflict"))
    }

    pub(crate) async fn inspect_ayni(
        &self,
        repository_id: &str,
    ) -> Result<v1::InspectAyniRepositoryResponse, Status> {
        let lookup_id = repository_id.to_owned();
        let repository = with_store(self, move |s| s.repository_connection(&lookup_id))
            .await
            .map_err(|_| Status::not_found("repository_not_found"))?;
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
        .map_err(|_| Status::internal("repository_snapshot_failed"))?;
        let mut result = v1::InspectAyniRepositoryResponse {
            repository_snapshot_id: snapshot.id,
            repository_revision: head.clone(),
            ayni_version: ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION.into(),
            workflow_template_version: onboarding_templates::TEMPLATE_VERSION.into(),
            oci_repository: format!(
                "ghcr.io/{}-ayni",
                format!("{}/{}", github.owner, github.name).to_lowercase()
            ),
            workflows: onboarding_templates::render(
                onboarding_templates::TEMPLATE_VERSION,
                &repository.default_branch,
            )
            .map_err(|_| Status::internal("template_invalid"))?
            .files
            .into_iter()
            .map(|(path, contents)| v1::WorkflowProposal {
                path: path.into(),
                contents,
            })
            .collect(),
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
        inspect_lock(&lock, &mut result);
        Ok(result)
    }

    /// Resumes setup operations independently from browser sessions.
    pub async fn reconcile_onboarding(&self) {
        if self.source.is_none() {
            return;
        }
        if let Err(reason) = self.reconcile_setup_actions().await {
            tracing::warn!(reason_code = reason, "setup reconciliation deferred");
        }
    }
}

fn inspect_lock(bytes: &[u8], result: &mut v1::InspectAyniRepositoryResponse) {
    use ayni_orchestrator_domain::digest as assessment;
    use ayni_orchestrator_profile_ayni::ayni_release;
    let Ok(lock) = serde_json::from_slice::<Value>(bytes) else {
        result.reason = "lock_invalid".into();
        return;
    };
    let fingerprint = lock["fingerprint"].as_str().unwrap_or("");
    result.lock_compatible = assessment::digest(fingerprint)
        && lock["ayni_version"] == ayni_release::CLI_VERSION
        && lock["schema_version"] == "0.9.0";
    if result.lock_compatible {
        result.lock_fingerprint = fingerprint.trim_start_matches("sha256:").into();
    }
    // This is a pinned-policy observation, never certificate verification.
    result.trust_policy_pinned = lock["certificate_trust_policy"]["fingerprint"]
        .as_str()
        .is_some_and(|p| assessment::digest(p) && p != assessment::sha256(b"{}"));
    result.reason = if !result.policy_present {
        "policy_missing"
    } else if !result.lock_compatible {
        "lock_incompatible"
    } else if !result.trust_policy_pinned {
        "trust_policy_missing"
    } else {
        "signing_configuration_requires_operator"
    }
    .into();
}

#[path = "setup.rs"]
mod onboarding_setup;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspection_distinguishes_compatibility_from_certificate_verification()
    -> Result<(), serde_json::Error> {
        let mut inspection = v1::InspectAyniRepositoryResponse {
            policy_present: true,
            ..Default::default()
        };
        let mut lock = serde_json::json!({"schema_version":"0.9.0","ayni_version":ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION,"fingerprint":format!("sha256:{}","a".repeat(64)),"certificate_trust_policy":{"fingerprint":ayni_orchestrator_domain::digest::sha256(b"{}")}});
        inspect_lock(&serde_json::to_vec(&lock)?, &mut inspection);
        assert!(inspection.lock_compatible);
        assert!(!inspection.trust_policy_pinned);
        assert_eq!(inspection.reason, "trust_policy_missing");
        lock["ayni_version"] = "unsupported".into();
        inspect_lock(&serde_json::to_vec(&lock)?, &mut inspection);
        assert!(!inspection.lock_compatible);
        assert_eq!(inspection.reason, "lock_incompatible");
        Ok(())
    }
}
