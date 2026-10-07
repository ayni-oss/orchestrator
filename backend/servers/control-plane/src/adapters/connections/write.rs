use super::{
    AppState, BTreeMap, ConnectionSecrets, SourceClient, Status, Uuid, docker, retire_candidate,
    retired, secrets, store_error, v1, with_store_async,
};
impl AppState {
    pub(super) async fn write_connection(
        &self,
        mut value: v1::Connection,
        credentials: Option<v1::ConnectionCredentials>,
        key: String,
        operation: &'static str,
    ) -> Result<v1::Connection, Status> {
        ayni_orchestrator_api::validation::idempotency_key(&key)
            .map_err(|_| Status::invalid_argument("idempotency_key_invalid"))?;
        let credentials =
            credentials.ok_or_else(|| Status::invalid_argument("credentials_required"))?;
        validate_credentials(&value, &credentials)?;
        let adapter = secrets()?;
        let name = format!("ayni-connection-{}", Uuid::now_v7().simple());
        let previous = value.clone();
        prepare_candidate(&mut value, &name);
        let intent = value.clone();
        let fresh = with_store_async(self, move |store| {
            store.connection_intent(&key, &intent, operation)
        })
        .await
        .map_err(store_error)?;
        if !fresh {
            return self.connection(value.id).await;
        }
        let candidate = self
            .create_candidate_secrets(&mut value, &credentials, &adapter, &name)
            .await?;
        if candidate.kind == "ghcr" {
            return Ok(value);
        }
        self.finish_github_replacement(previous, value, candidate)
            .await
    }
    async fn create_candidate_secrets(
        &self,
        value: &mut v1::Connection,
        credentials: &v1::ConnectionCredentials,
        adapter: &ConnectionSecrets,
        name: &str,
    ) -> Result<v1::Connection, Status> {
        let data = if value.kind == "github_app" {
            BTreeMap::from([
                ("app-id".into(), value.app_id.clone()),
                ("private-key.pem".into(), credentials.private_key.clone()),
            ])
        } else {
            docker(&value.username, &credentials.token)?
        };
        let (uid, revision) = adapter
            .create(name, &value.id, data, value.kind == "ghcr")
            .await
            .map_err(Status::unavailable)?;
        value.candidate_secret_uid.clone_from(&uid);
        self.persist_connection(value.clone()).await?;
        let mut candidate = value.clone();
        candidate.secret_name = name.into();
        candidate.secret_uid = uid;
        candidate.observed_revision = revision;
        if candidate.kind == "ghcr" {
            let pull = format!("{name}-pull");
            let (pull_uid, _) = adapter
                .create(
                    &pull,
                    &value.id,
                    docker(&value.username, &credentials.pull_token)?,
                    true,
                )
                .await
                .map_err(Status::unavailable)?;
            candidate.pull_secret_name.clone_from(&pull);
            candidate.pull_secret_uid.clone_from(&pull_uid);
            value.candidate_pull_secret_name = pull;
            value.candidate_pull_secret_uid = pull_uid;
        }
        self.persist_connection(value.clone()).await?;
        Ok(candidate)
    }
    async fn finish_github_replacement(
        &self,
        previous: v1::Connection,
        mut value: v1::Connection,
        mut candidate: v1::Connection,
    ) -> Result<v1::Connection, Status> {
        if let Err(error) = self.validate_connection(&candidate).await {
            value.secret_name = previous.secret_name;
            value.secret_uid = previous.secret_uid;
            value.observed_revision = previous.observed_revision;
            value.status = if value.secret_name.is_empty() {
                "failed"
            } else {
                "ready"
            }
            .into();
            value.failure_reason = error.message().into();
            retire_candidate(&mut value);
            return self.persist_connection(value).await;
        }
        if !previous.secret_name.is_empty() {
            candidate.retired_secret_revisions.push(retired(&previous));
        }
        clear_candidate(&mut candidate);
        candidate.status = "ready".into();
        candidate
            .desired_revision
            .clone_from(&candidate.observed_revision);
        candidate.failure_reason.clear();
        self.persist_connection(candidate.clone()).await?;
        self.activate_github(&candidate).await?;
        Ok(candidate)
    }
}
fn validate_credentials(
    value: &v1::Connection,
    credentials: &v1::ConnectionCredentials,
) -> Result<(), Status> {
    if credentials.private_key.len() > 32768
        || credentials.token.len() > 4096
        || credentials.pull_token.len() > 4096
    {
        return Err(Status::invalid_argument("credentials_too_large"));
    }
    if value.kind == "github_app" {
        let candidate = SourceClient::unconfigured(20).map_err(Status::unavailable)?;
        candidate
            .activate(value.app_id.clone(), credentials.private_key.as_bytes())
            .map_err(Status::invalid_argument)?;
    }
    if value.kind == "ghcr"
        && (credentials.token.is_empty()
            || credentials.pull_token.is_empty()
            || credentials.token == credentials.pull_token)
    {
        return Err(Status::invalid_argument(
            "distinct_push_and_pull_credentials_required",
        ));
    }
    Ok(())
}
fn prepare_candidate(value: &mut v1::Connection, name: &str) {
    if !value.candidate_secret_name.is_empty() {
        value
            .retired_secret_revisions
            .push(v1::ConnectionSecretRevision {
                secret_name: value.candidate_secret_name.clone(),
                secret_uid: value.candidate_secret_uid.clone(),
                pull_secret_name: value.candidate_pull_secret_name.clone(),
                pull_secret_uid: value.candidate_pull_secret_uid.clone(),
            });
    }
    value.desired_revision = name.into();
    value.candidate_secret_name = name.into();
    value.candidate_secret_uid.clear();
    if value.kind == "ghcr" {
        value.candidate_pull_secret_name = format!("{name}-pull");
        value.candidate_pull_secret_uid.clear();
    }
    value.status = "pending".into();
    value.failure_reason.clear();
}
pub(super) fn clear_candidate(value: &mut v1::Connection) {
    value.candidate_secret_name.clear();
    value.candidate_secret_uid.clear();
    value.candidate_pull_secret_name.clear();
    value.candidate_pull_secret_uid.clear();
}
