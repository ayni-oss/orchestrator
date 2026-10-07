use super::{
    AppState, SourceClient, Status, retire_candidate, retired, secrets, store_error, v1,
    with_store_async,
};
impl AppState {
    pub(super) async fn finish_connection_deletion(
        &self,
        value: &v1::Connection,
    ) -> Result<(), Status> {
        self.cleanup_connection_secrets(value).await?;
        let id = value.id.clone();
        with_store_async(self, move |store| store.delete_connection(&id))
            .await
            .map_err(store_error)?;
        if value.kind == "github_app" {
            self.disable_github();
        }
        Ok(())
    }
    pub(super) fn disable_github(&self) {
        if let Some(source) = self.source.as_ref() {
            source.disable();
        }
    }
    pub(super) async fn cleanup_retired_credentials(
        &self,
        value: &mut v1::Connection,
    ) -> Result<Option<Status>, Status> {
        if value.management_source != "ui" || value.retired_secret_revisions.is_empty() {
            return Ok(None);
        }
        let mut retained = Vec::new();
        let mut failure = None;
        for revision in value.retired_secret_revisions.clone() {
            match self.cleanup_retired_revision(&value.id, &revision).await {
                Ok(true) => {}
                Ok(false) => retained.push(revision),
                Err(error) => {
                    retained.push(revision);
                    tracing::warn!(
                        reason_code = error.message(),
                        connection_id = value.id,
                        "retired credential cleanup failed"
                    );
                    failure = Some(error);
                }
            }
        }
        value.retired_secret_revisions = retained;
        self.persist_connection(value.clone()).await?;
        Ok(failure)
    }
    async fn cleanup_retired_revision(
        &self,
        id: &str,
        revision: &v1::ConnectionSecretRevision,
    ) -> Result<bool, Status> {
        if self.credential_revision_in_use(revision).await? {
            return Ok(false);
        }
        let adapter = secrets()?;
        adapter
            .delete(&revision.secret_name, id, &revision.secret_uid)
            .await
            .map_err(Status::failed_precondition)?;
        if !revision.pull_secret_name.is_empty() {
            adapter
                .delete(&revision.pull_secret_name, id, &revision.pull_secret_uid)
                .await
                .map_err(Status::failed_precondition)?;
        }
        Ok(true)
    }
    pub(super) async fn reconcile_connection(
        &self,
        snapshot: v1::Connection,
    ) -> Result<(), Status> {
        let github_guard = if snapshot.kind == "github_app" {
            Some(self.connection_lock.lock().await)
        } else {
            None
        };
        let mut value = self.connection(snapshot.id).await?;
        // An explicit administrator declaration cannot take over a UI-owned App.
        if value.failure_reason == "github_admin_identity_conflict"
            && std::env::var("ORCHESTRATOR_GITHUB_APP_SECRET_NAME")
                .is_ok_and(|name| !name.is_empty())
        {
            self.disable_github();
            return Ok(());
        }
        if value.status == "deleting" {
            return self.finish_connection_deletion(&value).await;
        }
        let cleanup_failure = self.cleanup_retired_credentials(&mut value).await?;
        let id = value.id.clone();
        if value.kind == "github_app" {
            self.reconcile_github_candidate(&mut value).await?;
            self.reconcile_github_active(value).await?;
        } else {
            self.reconcile_registry(value).await?;
        }
        // Drop the GitHub mutation guard before recording an independent cleanup observation.
        drop(github_guard);
        if let Some(error) = cleanup_failure {
            self.record_connection_reconciliation_failure(&id, &error)
                .await;
        }
        Ok(())
    }
    pub(super) async fn record_connection_reconciliation_failure(&self, id: &str, error: &Status) {
        let reason = crate::truncate_utf8(error.message(), 128);
        tracing::warn!(
            reason_code = reason,
            connection_id = id,
            "connection reconciliation failed"
        );
        let _guard = self.connection_lock.lock().await;
        if let Ok(mut current) = self.connection(id.into()).await {
            if current.failure_reason.is_empty() {
                current.failure_reason = reason;
                let _ = self.persist_connection(current).await;
            }
        }
    }
    async fn credential_revision_in_use(
        &self,
        revision: &v1::ConnectionSecretRevision,
    ) -> Result<bool, Status> {
        for name in [&revision.secret_name, &revision.pull_secret_name] {
            if name.is_empty() {
                continue;
            }
            let copied = name.clone();
            let durable =
                with_store_async(self, move |store| store.connection_secret_in_use(&copied))
                    .await
                    .map_err(store_error)?;
            if durable || secrets()?.in_use(name).await.map_err(Status::unavailable)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) async fn reconcile_github_candidate(
        &self,
        value: &mut v1::Connection,
    ) -> Result<(), Status> {
        if !value.enabled || value.candidate_secret_name.is_empty() {
            return Ok(());
        }
        let mut candidate = value.clone();
        candidate
            .secret_name
            .clone_from(&value.candidate_secret_name);
        candidate.secret_uid.clone_from(&value.candidate_secret_uid);
        if let Ok(credential) = secrets()?
            .github(
                &candidate.secret_name,
                Some((&candidate.id, &candidate.secret_uid)),
            )
            .await
        {
            candidate.secret_uid.clone_from(&credential.uid);
            value.candidate_secret_uid = credential.uid;
            candidate.observed_revision = credential.revision;
        }
        match self.validate_connection(&candidate).await {
            Ok(()) => {
                if !value.secret_name.is_empty() && value.secret_name != candidate.secret_name {
                    candidate.retired_secret_revisions.push(retired(value));
                }
                candidate.status = "ready".into();
                candidate.candidate_secret_name.clear();
                candidate.candidate_secret_uid.clear();
                candidate.failure_reason.clear();
                *value = candidate;
            }
            Err(error) => {
                value.failure_reason = error.message().into();
                value.status = if value.secret_name.is_empty() {
                    "failed"
                } else {
                    "ready"
                }
                .into();
                retire_candidate(value);
            }
        }
        self.persist_connection(value.clone()).await?;
        Ok(())
    }
    pub(super) async fn reconcile_github_active(
        &self,
        mut value: v1::Connection,
    ) -> Result<(), Status> {
        if !value.enabled {
            return self.activate_github(&value).await;
        }
        let owner = (value.management_source == "ui")
            .then_some((value.id.as_str(), value.secret_uid.as_str()));
        let credential = match secrets()?.github(&value.secret_name, owner).await {
            Ok(credential) => credential,
            Err(reason) => return self.fail_github(value, reason).await,
        };
        if credential.revision == value.observed_revision && value.status == "ready" {
            return self.activate_github(&value).await;
        }
        if !value.app_id.is_empty() && value.app_id != credential.app_id {
            return self
                .fail_github(value, "github_app_identity_mismatch")
                .await;
        }
        let candidate = SourceClient::unconfigured(20).map_err(Status::unavailable)?;
        if let Err(reason) = candidate.activate(credential.app_id.clone(), &credential.private_key)
        {
            return self.fail_github(value, reason).await;
        }
        if let Err(reason) = candidate.validate_app().await {
            return self.fail_github(value, reason).await;
        }
        value.app_id = credential.app_id;
        value.secret_uid = credential.uid;
        value.observed_revision = credential.revision;
        value.desired_revision.clone_from(&value.observed_revision);
        value.status = "ready".into();
        value.failure_reason.clear();
        self.activate_github(&value).await?;
        self.persist_connection(value).await?;
        Ok(())
    }
    async fn fail_github(&self, mut value: v1::Connection, reason: &str) -> Result<(), Status> {
        value.status = "failed".into();
        value.failure_reason = reason.into();
        self.disable_github();
        self.persist_connection(value).await?;
        Ok(())
    }
}
