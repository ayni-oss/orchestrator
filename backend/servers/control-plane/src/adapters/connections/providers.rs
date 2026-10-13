//! LLM connections share immutable Secret revision and guarded activation semantics.
use super::{AppState, Status, retire_candidate, retired, secrets, v1};
use crate::ModelCatalog;

impl AppState {
    pub(crate) async fn provider_catalog(
        &self,
        connection: &v1::Connection,
    ) -> Result<ModelCatalog, Status> {
        if connection.kind != "openai" {
            return Err(Status::failed_precondition("provider_not_available"));
        }
        let cache_key = format!(
            "{}:{}:{}",
            connection.id, connection.secret_uid, connection.observed_revision
        );
        let cached = self.provider_catalogs.lock().await.get(&cache_key).cloned();
        if let Some(catalog) = cached {
            return Ok(catalog);
        }
        let owner = (connection.management_source == "ui")
            .then_some((connection.id.as_str(), connection.secret_uid.as_str()));
        let key = secrets()?
            .openai(&connection.secret_name, owner)
            .await
            .map_err(Status::failed_precondition)?;
        let catalog = ModelCatalog::openai(&key)
            .map_err(|_| Status::failed_precondition("provider_credentials_invalid"))?;
        let mut cache = self.provider_catalogs.lock().await;
        // Keep only the currently accessed revision per connection. Frozen work mounts its own key.
        cache.retain(|key, _| !key.starts_with(&format!("{}:", connection.id)));
        cache.insert(cache_key, catalog.clone());
        drop(cache);
        Ok(catalog)
    }

    pub(crate) async fn catalog_for_connection(&self, id: &str) -> Result<ModelCatalog, Status> {
        if id.is_empty() {
            return self
                .model_catalog()
                .ok_or_else(|| Status::unavailable("provider_unavailable"));
        }
        let value = self.connection(id.into()).await?;
        if value.kind != "openai" || !value.enabled || value.status != "ready" {
            return Err(Status::failed_precondition("provider_connection_not_ready"));
        }
        self.provider_catalog(&value).await
    }

    pub(crate) async fn freeze_provider_credentials(
        &self,
        id: &str,
        model: &str,
    ) -> Result<v1::FrozenProviderCredentials, Status> {
        let value = self.connection(id.into()).await?;
        let catalog = self.catalog_for_connection(id).await?;
        if !catalog
            .contains_current(model)
            .await
            .map_err(|_| Status::unavailable("provider_unavailable"))?
        {
            return Err(Status::failed_precondition("provider_model_unavailable"));
        }
        Ok(v1::FrozenProviderCredentials {
            connection_id: value.id,
            credential_revision: value.observed_revision,
            secret_name: value.secret_name,
            secret_uid: value.secret_uid,
        })
    }

    pub(super) async fn reconcile_provider(&self, mut value: v1::Connection) -> Result<(), Status> {
        if !value.enabled {
            return Ok(());
        }
        if !value.candidate_secret_name.is_empty() {
            let mut candidate = value.clone();
            candidate
                .secret_name
                .clone_from(&value.candidate_secret_name);
            let (uid, revision) = secrets()?
                .metadata(
                    &candidate.secret_name,
                    Some((&candidate.id, &candidate.candidate_secret_uid)),
                )
                .await
                .map_err(Status::failed_precondition)?;
            candidate.secret_uid = uid;
            candidate.observed_revision = revision;
            if let Err(error) = self.validate_connection(&candidate).await {
                value.status = if value.secret_name.is_empty() {
                    "failed"
                } else {
                    "ready"
                }
                .into();
                value.failure_reason = error.message().into();
                retire_candidate(&mut value);
            } else {
                if !value.secret_name.is_empty() {
                    candidate.retired_secret_revisions.push(retired(&value));
                }
                super::write::clear_candidate(&mut candidate);
                candidate.status = "ready".into();
                candidate
                    .desired_revision
                    .clone_from(&candidate.observed_revision);
                candidate.failure_reason.clear();
                value = candidate;
            }
            self.persist_connection(value.clone()).await?;
        }
        if value.secret_name.is_empty() {
            return Ok(());
        }
        let owner = (value.management_source == "ui")
            .then_some((value.id.as_str(), value.secret_uid.as_str()));
        let (uid, revision) = secrets()?
            .metadata(&value.secret_name, owner)
            .await
            .map_err(Status::failed_precondition)?;
        if value.status == "ready" && value.observed_revision == revision {
            return Ok(());
        }
        value.secret_uid = uid;
        value.observed_revision = revision;
        match self.validate_connection(&value).await {
            Ok(()) => {
                value.status = "ready".into();
                value.failure_reason.clear();
                if value.management_source == "ui" {
                    value.desired_revision.clone_from(&value.observed_revision);
                }
            }
            Err(error) => {
                value.status = "failed".into();
                value.failure_reason = error.message().into();
            }
        }
        self.persist_connection(value).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AvailableModel;

    #[tokio::test]
    async fn model_selection_uses_the_selected_connection_and_blocks_disabled_work()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let state = AppState::open(directory.path().join("state.db"), 3)?;
        let mut value = v1::Connection {
            id: "openai-one".into(),
            kind: "openai".into(),
            management_source: "ui".into(),
            secret_name: "secret-one".into(),
            secret_uid: "uid-one".into(),
            observed_revision: "one".into(),
            status: "ready".into(),
            enabled: true,
            resource_version: 1,
            ..Default::default()
        };
        state.persist_connection(value.clone()).await?;
        let catalog = ModelCatalog::fixed(vec![AvailableModel {
            id: "gpt-5.6-sol".into(),
            name: None,
            owned_by: None,
            created_at: None,
            context_length: None,
            max_output_tokens: None,
        }]);
        state
            .provider_catalogs
            .lock()
            .await
            .insert("openai-one:uid-one:one".into(), catalog);
        let binding = state
            .freeze_provider_credentials(&value.id, "gpt-5.6-sol")
            .await?;
        assert_eq!(binding.secret_name, "secret-one");
        assert_eq!(binding.credential_revision, "one");
        assert!(
            state
                .freeze_provider_credentials(&value.id, "unknown")
                .await
                .is_err()
        );
        assert!(state.catalog_for_connection("openai-other").await.is_err());
        value.enabled = false;
        value.resource_version += 1;
        state.persist_connection(value.clone()).await?;
        assert!(state.catalog_for_connection(&value.id).await.is_err());
        assert_eq!(binding.secret_name, "secret-one");
        state.shutdown_database().await?;
        Ok(())
    }
}
