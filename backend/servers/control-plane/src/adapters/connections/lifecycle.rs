use super::{AppState, Status, retired, secrets, store_error, v1, with_store_async};
impl AppState {
    pub(super) async fn cleanup_connection_secrets(
        &self,
        value: &v1::Connection,
    ) -> Result<(), Status> {
        let adapter = secrets()?;
        self.verify_no_connection_consumers(value).await?;
        for revision in revisions(value) {
            adapter
                .delete(&revision.secret_name, &value.id, &revision.secret_uid)
                .await
                .map_err(Status::failed_precondition)?;
            if !revision.pull_secret_name.is_empty() {
                adapter
                    .delete(
                        &revision.pull_secret_name,
                        &value.id,
                        &revision.pull_secret_uid,
                    )
                    .await
                    .map_err(Status::failed_precondition)?;
            }
        }
        Ok(())
    }
    pub(super) async fn verify_no_connection_consumers(
        &self,
        value: &v1::Connection,
    ) -> Result<(), Status> {
        let adapter = secrets()?;
        for revision in revisions(value) {
            for name in [&revision.secret_name, &revision.pull_secret_name] {
                if name.is_empty() {
                    continue;
                }
                let secret_name = name.clone();
                let used = with_store_async(self, move |store| {
                    store.connection_secret_in_use(&secret_name)
                })
                .await
                .map_err(store_error)?;
                if used || adapter.in_use(name).await.map_err(Status::unavailable)? {
                    return Err(Status::failed_precondition("connection_has_consumers"));
                }
            }
        }
        Ok(())
    }
    pub(super) async fn replayed(
        &self,
        id: String,
        key: String,
        version: u64,
        operation: String,
    ) -> Result<bool, Status> {
        ayni_orchestrator_api::validation::idempotency_key(&key)
            .map_err(|_| Status::invalid_argument("idempotency_key_invalid"))?;
        with_store_async(self, move |store| {
            store.connection_operation_replayed(&key, &id, version, &operation)
        })
        .await
        .map_err(store_error)
    }
}

fn revisions(value: &v1::Connection) -> Vec<v1::ConnectionSecretRevision> {
    let mut revisions = value.retired_secret_revisions.clone();
    if !value.secret_name.is_empty() {
        revisions.push(retired(value));
    }
    if !value.candidate_secret_name.is_empty() {
        revisions.push(v1::ConnectionSecretRevision {
            secret_name: value.candidate_secret_name.clone(),
            secret_uid: value.candidate_secret_uid.clone(),
            pull_secret_name: value.candidate_pull_secret_name.clone(),
            pull_secret_uid: value.candidate_pull_secret_uid.clone(),
        });
    }
    revisions
}
