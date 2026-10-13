use super::{AppState, Status, v1};
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdminRegistry {
    id: String,
    display_name: String,
    image_namespace: String,
    username: String,
    pull_secret_name: String,
    push_secret_name: String,
}
impl AppState {
    pub(super) async fn register_admin_connections(&self) -> Result<(), Status> {
        self.register_admin_github(std::env::var("ORCHESTRATOR_GITHUB_APP_SECRET_NAME").ok())
            .await?;
        if let Ok(configuration) = std::env::var("ORCHESTRATOR_ADMIN_REGISTRY_CONNECTIONS") {
            let configured: Vec<AdminRegistry> = serde_json::from_str(&configuration)
                .map_err(|_| Status::failed_precondition("admin_registry_configuration_invalid"))?;
            for registry in configured {
                if !crate::registry_probe::valid_namespace(&registry.image_namespace)
                    || registry.id.is_empty()
                    || registry.id.len() > 100
                    || registry.push_secret_name.is_empty()
                    || registry.pull_secret_name.is_empty()
                    || registry.push_secret_name == registry.pull_secret_name
                {
                    return Err(Status::failed_precondition(
                        "admin_registry_configuration_invalid",
                    ));
                }
                match self.connection(registry.id.clone()).await {
                    Ok(mut existing) => {
                        if existing.management_source != "admin" || existing.kind != "ghcr" {
                            return Err(Status::failed_precondition(
                                "admin_registry_identity_conflict",
                            ));
                        }
                        if !existing.enabled
                            || (
                                &existing.secret_name,
                                &existing.pull_secret_name,
                                &existing.image_namespace,
                                &existing.username,
                                &existing.display_name,
                            ) != (
                                &registry.push_secret_name,
                                &registry.pull_secret_name,
                                &registry.image_namespace,
                                &registry.username,
                                &registry.display_name,
                            )
                        {
                            existing.secret_name = registry.push_secret_name;
                            existing.pull_secret_name = registry.pull_secret_name;
                            existing.image_namespace = registry.image_namespace;
                            existing.username = registry.username;
                            existing.display_name = registry.display_name;
                            existing.enabled = true;
                            existing.status = "pending".into();
                            existing.resource_version += 1;
                            self.persist_connection(existing).await?;
                        }
                    }
                    Err(_) => {
                        self.persist_connection(v1::Connection {
                            id: registry.id,
                            kind: "ghcr".into(),
                            management_source: "admin".into(),
                            display_name: registry.display_name,
                            image_namespace: registry.image_namespace,
                            username: registry.username,
                            secret_name: registry.push_secret_name,
                            pull_secret_name: registry.pull_secret_name,
                            enabled: true,
                            status: "pending".into(),
                            resource_version: 1,
                            ..Default::default()
                        })
                        .await?;
                    }
                }
            }
        }
        self.register_admin_openai().await?;
        self.sync_admin_ownership().await?;
        Ok(())
    }
    pub(super) async fn register_admin_github(&self, name: Option<String>) -> Result<(), Status> {
        let Some(name) = name.filter(|name| !name.is_empty()) else {
            return Ok(());
        };
        let _guard = self.connection_lock.lock().await;
        if let Some(mut existing) = self
            .connections()
            .await?
            .into_iter()
            .find(|connection| connection.kind == "github_app")
        {
            if existing.management_source != "admin" {
                existing.status = "failed".into();
                existing.failure_reason = "github_admin_identity_conflict".into();
                self.persist_connection(existing).await?;
                if let Some(source) = self.source.as_ref() {
                    source.disable();
                }
                return Err(Status::failed_precondition(
                    "github_admin_identity_conflict",
                ));
            }
            return Ok(());
        }
        self.persist_connection(v1::Connection {
            id: "github-admin".into(),
            kind: "github_app".into(),
            management_source: "admin".into(),
            display_name: "GitHub App".into(),
            secret_name: name,
            enabled: true,
            resource_version: 1,
            status: "pending".into(),
            ..Default::default()
        })
        .await?;
        Ok(())
    }
    async fn register_admin_openai(&self) -> Result<(), Status> {
        let Ok(source_name) = std::env::var("ORCHESTRATOR_OPENAI_SECRET_NAME") else {
            return Ok(());
        };
        if source_name.is_empty() {
            return Ok(());
        }
        let _guard = self.connection_lock.lock().await;
        let mut value = match self.connection("openai-admin".into()).await {
            Ok(value) if value.management_source == "admin" && value.kind == "openai" => value,
            Ok(_) => {
                return Err(Status::failed_precondition(
                    "provider_admin_identity_conflict",
                ));
            }
            Err(_) => v1::Connection {
                id: "openai-admin".into(),
                kind: "openai".into(),
                management_source: "admin".into(),
                display_name: "OpenAI".into(),
                resource_version: 1,
                ..Default::default()
            },
        };
        let (name, uid, revision, source_revision) = super::secrets()?
            .snapshot_openai(&source_name, &value.id)
            .await
            .map_err(Status::failed_precondition)?;
        if value.secret_name != name || !value.enabled {
            let mut candidate = value.clone();
            candidate.secret_name = name;
            candidate.secret_uid = uid;
            candidate.observed_revision = revision;
            candidate.desired_revision = source_revision.clone();
            candidate.enabled = true;
            if let Err(error) = self.validate_connection(&candidate).await {
                if value.secret_name.is_empty() {
                    candidate.status = "failed".into();
                    candidate.failure_reason = error.message().into();
                    self.persist_connection(candidate).await?;
                } else if value.failure_reason != error.message()
                    || value.desired_revision != source_revision
                {
                    value.resource_version += 1;
                    value.failure_reason = error.message().into();
                    value.desired_revision = source_revision;
                    self.persist_connection(value).await?;
                }
                return Ok(());
            }
            if !value.secret_name.is_empty() {
                candidate
                    .retired_secret_revisions
                    .push(super::retired(&value));
                candidate.resource_version += 1;
            }
            candidate.status = "ready".into();
            candidate.failure_reason.clear();
            self.persist_connection(candidate).await?;
        }
        Ok(())
    }
    async fn sync_admin_ownership(&self) -> Result<(), Status> {
        let github_secret =
            std::env::var("ORCHESTRATOR_GITHUB_APP_SECRET_NAME").unwrap_or_default();
        let configured_registries: serde_json::Value = serde_json::from_str(
            &std::env::var("ORCHESTRATOR_ADMIN_REGISTRY_CONNECTIONS")
                .unwrap_or_else(|_| "[]".into()),
        )
        .map_err(|_| Status::failed_precondition("admin_registry_configuration_invalid"))?;
        for mut existing in self.connections().await? {
            if existing.management_source != "admin" {
                continue;
            }
            let configured = if existing.kind == "github_app" {
                !github_secret.is_empty()
            } else if existing.kind == "openai" {
                std::env::var("ORCHESTRATOR_OPENAI_SECRET_NAME").is_ok_and(|name| !name.is_empty())
            } else {
                configured_registries.as_array().is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item["id"].as_str() == Some(existing.id.as_str()))
                })
            };
            if !configured && existing.enabled {
                existing.enabled = false;
                existing.status = "disabled".into();
                existing.resource_version += 1;
                self.persist_connection(existing).await?;
            } else if configured
                && existing.kind == "github_app"
                && (existing.secret_name != github_secret || !existing.enabled)
            {
                existing.secret_name = github_secret.clone();
                existing.enabled = true;
                existing.status = "pending".into();
                existing.observed_revision.clear();
                existing.resource_version += 1;
                self.persist_connection(existing).await?;
            }
        }
        Ok(())
    }
}
