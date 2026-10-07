//! Trusted connection operations. Durable records contain references, never credentials.
#![allow(
    clippy::collapsible_if,
    clippy::single_match_else,
    reason = "explicit connection lifecycle branches retain failure and cleanup context"
)]
use crate::{AppState, adapters::github::SourceClient, coordinators::with_store_async};
use ayni_orchestrator_api::v1;
use ayni_orchestrator_kubernetes::connections::ConnectionSecrets;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use tonic::Status;
use uuid::Uuid;
mod admin;
mod lifecycle;
mod reconciliation;
mod registry;
mod write;

fn secrets() -> Result<ConnectionSecrets, Status> {
    let namespace = std::env::var("ORCHESTRATOR_CONNECTION_NAMESPACE")
        .map_err(|_| Status::failed_precondition("connection_namespace_unconfigured"))?;
    ConnectionSecrets::new(&namespace).map_err(Status::unavailable)
}
fn store_error(_: crate::adapters::sqlite::CleanStoreError) -> Status {
    Status::failed_precondition("connection_state_conflict")
}
impl AppState {
    pub(crate) async fn connections(&self) -> Result<Vec<v1::Connection>, Status> {
        with_store_async(self, |store| store.list_connections())
            .await
            .map_err(store_error)
    }
    pub(crate) async fn connection(&self, id: String) -> Result<v1::Connection, Status> {
        with_store_async(self, move |store| store.get_connection(&id))
            .await
            .map_err(store_error)
    }
    async fn persist_connection(&self, value: v1::Connection) -> Result<v1::Connection, Status> {
        let saved = value.clone();
        with_store_async(self, move |store| store.save_connection(&saved))
            .await
            .map_err(store_error)?;
        Ok(value)
    }
    pub(crate) async fn create_connection(
        &self,
        request: v1::CreateConnectionRequest,
    ) -> Result<v1::Connection, Status> {
        let _guard = self.connection_lock.lock().await;
        ayni_orchestrator_api::validation::idempotency_key(&request.idempotency_key)
            .map_err(|_| Status::invalid_argument("idempotency_key_invalid"))?;
        if request.display_name.len() > 128 || request.display_name.is_empty() {
            return Err(Status::invalid_argument("connection_name_invalid"));
        }
        if !matches!(request.kind.as_str(), "github_app" | "ghcr") {
            return Err(Status::invalid_argument("connection_kind_invalid"));
        }
        if request.kind == "github_app"
            && std::env::var("ORCHESTRATOR_GITHUB_APP_SECRET_NAME")
                .is_ok_and(|value| !value.is_empty())
        {
            return Err(Status::permission_denied("connection_admin_managed"));
        }
        if request.kind == "github_app"
            && (request.app_id.parse::<u64>().is_err() || request.app_id == "0")
        {
            return Err(Status::invalid_argument("github_app_id_invalid"));
        }
        if request.kind == "ghcr"
            && !crate::registry_probe::valid_namespace(&request.image_namespace)
        {
            return Err(Status::invalid_argument("registry_namespace_invalid"));
        }
        // Stable ID makes disconnected retries converge without persisting secret-bearing requests.
        let id = format!(
            "connection-{:x}",
            Sha256::digest(request.idempotency_key.as_bytes())
        );
        let id = id[..43].to_owned();
        if let Ok(existing) = self.connection(id.clone()).await {
            if existing.kind != request.kind
                || existing.display_name != request.display_name
                || existing.app_id != request.app_id
                || existing.username != request.username
                || existing.image_namespace != request.image_namespace
            {
                return Err(Status::already_exists("idempotency_key_conflict"));
            }
            if existing.secret_name.is_empty() && existing.candidate_secret_uid.is_empty() {
                let mut retry = existing;
                retry.resource_version += 1;
                let resubmit_key = format!(
                    "{:x}",
                    Sha256::digest(format!(
                        "{}:resubmit:{}",
                        request.idempotency_key, retry.resource_version
                    ))
                );
                return Box::pin(self.write_connection(
                    retry,
                    request.credentials,
                    resubmit_key,
                    "resubmit",
                ))
                .await;
            }
            return Ok(existing);
        }
        let value = v1::Connection {
            id,
            kind: request.kind,
            management_source: "ui".into(),
            display_name: request.display_name,
            app_id: request.app_id,
            username: request.username,
            image_namespace: request.image_namespace,
            enabled: true,
            resource_version: 1,
            status: "pending".into(),
            ..Default::default()
        };
        Box::pin(self.write_connection(
            value,
            request.credentials,
            request.idempotency_key,
            "create",
        ))
        .await
    }
    pub(crate) async fn replace_connection(
        &self,
        request: v1::ReplaceConnectionCredentialsRequest,
    ) -> Result<v1::Connection, Status> {
        let _guard = self.connection_lock.lock().await;
        if self
            .replayed(
                request.id.clone(),
                request.idempotency_key.clone(),
                request.expected_resource_version + 1,
                "replace".into(),
            )
            .await?
        {
            return self.connection(request.id).await;
        }
        let mut value = self.connection(request.id).await?;
        mutable(&value, request.expected_resource_version)?;
        value.resource_version += 1;
        Box::pin(self.write_connection(
            value,
            request.credentials,
            request.idempotency_key,
            "replace",
        ))
        .await
    }
    async fn validate_connection(&self, value: &v1::Connection) -> Result<(), Status> {
        if value.kind == "github_app" {
            let credential = secrets()?
                .github(
                    &value.secret_name,
                    (value.management_source == "ui")
                        .then_some((value.id.as_str(), value.secret_uid.as_str())),
                )
                .await
                .map_err(Status::failed_precondition)?;
            let client = SourceClient::unconfigured(20).map_err(Status::unavailable)?;
            client
                .activate(value.app_id.clone(), &credential.private_key)
                .map_err(Status::failed_precondition)?;
            client
                .validate_app()
                .await
                .map_err(Status::failed_precondition)
        } else {
            let namespace = std::env::var("ORCHESTRATOR_CONNECTION_NAMESPACE")
                .map_err(|_| Status::failed_precondition("connection_namespace_unconfigured"))?;
            crate::adapters::ghcr::test_connection(&namespace, value)
                .await
                .map_err(Status::failed_precondition)
        }
    }
    async fn activate_github(&self, value: &v1::Connection) -> Result<(), Status> {
        if value.kind != "github_app" {
            return Ok(());
        }
        let Some(source) = self.source.as_ref() else {
            return Err(Status::unavailable("github_client_unavailable"));
        };
        if !value.enabled {
            source.disable();
            return Ok(());
        }
        let owner = (value.management_source == "ui")
            .then_some((value.id.as_str(), value.secret_uid.as_str()));
        let credential = secrets()?
            .github(&value.secret_name, owner)
            .await
            .map_err(Status::failed_precondition)?;
        source
            .activate(credential.app_id, &credential.private_key)
            .map_err(Status::failed_precondition)
    }
    pub(crate) async fn test_connection(
        &self,
        id: String,
        version: u64,
        key: String,
    ) -> Result<v1::Connection, Status> {
        let _guard = self.connection_lock.lock().await;
        if self
            .replayed(id.clone(), key.clone(), version + 1, "test".into())
            .await?
        {
            return self.connection(id).await;
        }
        let mut value = self.connection(id).await?;
        if value.resource_version != version {
            return Err(Status::aborted("connection_version_changed"));
        }
        value.resource_version += 1;
        if value.kind == "ghcr" {
            value.status = "pending".into();
            let intent = value.clone();
            with_store_async(self, move |store| {
                store.connection_intent(&key, &intent, "test")
            })
            .await
            .map_err(store_error)?;
            return Ok(value);
        }
        let intent = value.clone();
        let intent_key = key.clone();
        with_store_async(self, move |store| {
            store.connection_intent(&intent_key, &intent, "test")
        })
        .await
        .map_err(store_error)?;
        match self.validate_connection(&value).await {
            Ok(()) => {
                value.status = "ready".into();
                value.failure_reason.clear();
            }
            Err(error) => {
                value.status = "failed".into();
                value.failure_reason = error.message().into();
            }
        }
        self.persist_connection(value).await
    }
    pub(crate) async fn enable_connection(
        &self,
        id: String,
        version: u64,
        enabled: bool,
        key: String,
    ) -> Result<v1::Connection, Status> {
        let _guard = self.connection_lock.lock().await;
        let mut value = self.connection(id).await?;
        let operation = format!("enabled:{enabled}");
        if self
            .replayed(
                value.id.clone(),
                key.clone(),
                version + 1,
                operation.clone(),
            )
            .await?
        {
            return Ok(value);
        }
        mutable(&value, version)?;
        value.enabled = enabled;
        value.status = if enabled { "pending" } else { "disabled" }.into();
        value.resource_version += 1;
        if enabled && value.kind == "github_app" {
            self.validate_connection(&value).await?;
            value.status = "ready".into();
        }
        let intent = value.clone();
        with_store_async(self, move |store| {
            store.connection_intent(&key, &intent, &operation)
        })
        .await
        .map_err(store_error)?;
        self.activate_github(&value).await?;
        Ok(value)
    }
    pub(crate) async fn remove_connection(
        &self,
        id: String,
        version: u64,
        key: String,
    ) -> Result<(), Status> {
        let _guard = self.connection_lock.lock().await;
        if self
            .replayed(id.clone(), key.clone(), version + 1, "delete".into())
            .await?
            && self.connection(id.clone()).await.is_err()
        {
            return Ok(());
        }
        let mut value = self.connection(id.clone()).await?;
        mutable(&value, version)?;
        let check_id = id.clone();
        with_store_async(self, move |store| {
            store.check_connection_deletion(&check_id)
        })
        .await
        .map_err(store_error)?;
        self.verify_no_connection_consumers(&value).await?;
        value.resource_version += 1;
        value.status = "deleting".into();
        value.enabled = false;
        let intent = value.clone();
        with_store_async(self, move |store| {
            store.connection_intent(&key, &intent, "delete")
        })
        .await
        .map_err(store_error)?;
        with_store_async(self, move |store| store.check_connection_deletion(&id))
            .await
            .map_err(store_error)?;
        self.cleanup_connection_secrets(&value).await?;
        let id = value.id.clone();
        with_store_async(self, move |store| store.delete_connection(&id))
            .await
            .map_err(store_error)?;
        if value.kind == "github_app" {
            if let Some(source) = self.source.as_ref() {
                source.disable();
            }
        }
        Ok(())
    }

    /// Reconciles explicit connection references and pending credential operations.
    ///
    /// # Errors
    /// Returns a bounded status when persistence or Kubernetes access is unavailable.
    pub async fn reconcile_connections(&self) -> Result<(), Status> {
        self.register_admin_connections().await?;
        for snapshot in self.connections().await? {
            let _github_guard = if snapshot.kind == "github_app" {
                Some(self.connection_lock.lock().await)
            } else {
                None
            };
            let mut value = self.connection(snapshot.id).await?;
            if value.status == "deleting" {
                self.finish_connection_deletion(&value).await?;
                continue;
            }
            self.cleanup_retired_credentials(&mut value).await?;
            if value.kind == "github_app" {
                self.reconcile_github_candidate(&mut value).await?;
                self.reconcile_github_active(value).await?;
            } else {
                self.reconcile_registry(value).await?;
            }
        }
        Ok(())
    }
    pub(crate) async fn complete_manifest(
        &self,
        request: v1::CompleteGithubManifestRequest,
    ) -> Result<v1::Connection, Status> {
        if request.code.is_empty()
            || request.code.len() > 256
            || !request
                .code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(Status::invalid_argument("manifest_code_invalid"));
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("ayni-orchestrator")
            .build()
            .map_err(|_| Status::unavailable("github_unavailable"))?;
        let response = client
            .post(format!(
                "https://api.github.com/app-manifests/{}/conversions",
                request.code
            ))
            .send()
            .await
            .map_err(|_| Status::unavailable("github_unavailable"))?;
        if !response.status().is_success() {
            return Err(Status::failed_precondition("github_manifest_failed"));
        }
        if response
            .content_length()
            .is_some_and(|length| length > 65536)
        {
            return Err(Status::failed_precondition("github_manifest_invalid"));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| Status::failed_precondition("github_manifest_invalid"))?;
        if bytes.len() > 65536 {
            return Err(Status::failed_precondition("github_manifest_invalid"));
        }
        let data: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| Status::failed_precondition("github_manifest_invalid"))?;
        self.create_connection(v1::CreateConnectionRequest {
            idempotency_key: request.idempotency_key,
            kind: "github_app".into(),
            display_name: data["name"].as_str().unwrap_or("GitHub App").into(),
            app_id: data["id"]
                .as_u64()
                .ok_or_else(|| Status::failed_precondition("github_manifest_invalid"))?
                .to_string(),
            credentials: Some(v1::ConnectionCredentials {
                private_key: data["pem"]
                    .as_str()
                    .ok_or_else(|| Status::failed_precondition("github_manifest_invalid"))?
                    .into(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
    }
}
fn mutable(value: &v1::Connection, version: u64) -> Result<(), Status> {
    if value.management_source != "ui" {
        return Err(Status::permission_denied("connection_admin_managed"));
    }
    if value.resource_version != version {
        return Err(Status::aborted("connection_version_changed"));
    }
    Ok(())
}
fn docker(username: &str, token: &str) -> Result<BTreeMap<String, String>, Status> {
    use base64::Engine as _;
    if username.is_empty()
        || username.len() > 100
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(Status::invalid_argument("registry_username_invalid"));
    }
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("{username}:{token}"));
    Ok(BTreeMap::from([(
        ".dockerconfigjson".into(),
        serde_json::json!({"auths":{"ghcr.io":{"username":username,"password":token,"auth":auth}}})
            .to_string(),
    )]))
}

fn retired(value: &v1::Connection) -> v1::ConnectionSecretRevision {
    v1::ConnectionSecretRevision {
        secret_name: value.secret_name.clone(),
        secret_uid: value.secret_uid.clone(),
        pull_secret_name: value.pull_secret_name.clone(),
        pull_secret_uid: value.pull_secret_uid.clone(),
    }
}

fn retire_candidate(value: &mut v1::Connection) {
    if !value.candidate_secret_name.is_empty() {
        value
            .retired_secret_revisions
            .push(v1::ConnectionSecretRevision {
                secret_name: std::mem::take(&mut value.candidate_secret_name),
                secret_uid: std::mem::take(&mut value.candidate_secret_uid),
                pull_secret_name: std::mem::take(&mut value.candidate_pull_secret_name),
                pull_secret_uid: std::mem::take(&mut value.candidate_pull_secret_uid),
            });
    }
    value.desired_revision = value.observed_revision.clone();
}

#[cfg(test)]
mod tests;
