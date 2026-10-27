//! Write-only provider credentials and safe configuration metadata.
use super::{
    Operation, ProviderSettings, StoreError, WorkflowStore, locked, new_id, operation, revision,
    safe_json, serialize_key, validate_id, validate_key, validate_name,
};
// Not Debug or Serialize: callers must not export these values.
pub struct SaveProvider<'a> {
    pub key: &'a str,
    pub id: &'a str,
    pub expected: u64,
    pub name: &'a str,
    pub provider: &'a str,
    pub capability: &'a str,
    pub settings: ProviderSettings,
    pub credential: &'a str,
}
pub struct ProviderCandidate {
    pub operation: Operation,
    pub credential: String,
}
fn validate(input: &SaveProvider<'_>) -> Result<String, StoreError> {
    validate_key(input.key)?;
    validate_name(input.name, "display_name")?;
    if !input.id.is_empty() {
        validate_id(input.id, "connection_id")?;
    }
    if input.name.len() > 120 || input.credential.is_empty() || input.credential.len() > 32768 {
        return Err(StoreError::InvalidInput("provider credentials"));
    }
    match (input.provider, input.capability) {
        ("openai", "llm") if input.settings == ProviderSettings::default() => {
            if input.credential.len() > 4096
                || input
                    .credential
                    .bytes()
                    .any(|v| v.is_ascii_whitespace() || !v.is_ascii_graphic())
            {
                return Err(StoreError::InvalidInput("api_key"));
            }
        }
        ("github", "container_registry") => {
            let repo = input
                .settings
                .registry_destination
                .strip_prefix("ghcr.io/")
                .ok_or(StoreError::InvalidInput("registry_destination"))?;
            if repo.len() > 200
                || repo.split('/').count() < 2
                || repo.split('/').any(|p| {
                    p.is_empty()
                        || p.starts_with(['.', '-', '_'])
                        || !p.bytes().all(|b| {
                            b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b)
                        })
                })
            {
                return Err(StoreError::InvalidInput("registry_destination"));
            }
            for name in [&input.settings.push_username, &input.settings.pull_username] {
                if name.is_empty()
                    || name.len() > 100
                    || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    return Err(StoreError::InvalidInput("registry username"));
                }
            }
            let values: Vec<String> = serde_json::from_str(input.credential)
                .map_err(|_| StoreError::InvalidInput("registry credentials"))?;
            if values.len() != 2
                || values.iter().any(|v| {
                    v.is_empty() || v.len() > 4096 || !v.bytes().all(|b| b.is_ascii_graphic())
                })
            {
                return Err(StoreError::InvalidInput("registry credentials"));
            }
        }
        _ => return Err(StoreError::InvalidInput("provider capability")),
    }
    serde_json::to_string(&(
        input.id,
        input.expected,
        input.name,
        input.provider,
        input.capability,
        &input.settings,
    ))
    .map_err(|_| StoreError::CorruptState)
}
impl WorkflowStore {
    /// Retain a candidate without publishing credential values into receipts.
    /// # Errors
    /// Reject unsupported providers, stale writes and conflicting retries.
    pub async fn save_provider_connection(
        &self,
        input: SaveProvider<'_>,
    ) -> Result<Operation, StoreError> {
        let safe = validate(&input)?;
        let settings =
            serde_json::to_string(&input.settings).map_err(|_| StoreError::CorruptState)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        serialize_key(&tx, input.key).await?;
        if let Some(row) = tx
            .query_opt(
                "SELECT * FROM workflow_connection_operations WHERE idempotency_key=$1",
                &[&input.key],
            )
            .await?
        {
            if row.get::<_, String>("kind") != "save"
                || row.get::<_, String>("safe_request") != safe
            {
                return Err(StoreError::IdempotencyConflict);
            }
            let stored: String = tx
                .query_one(
                    "SELECT value FROM workflow_connection_credentials WHERE revision_id=$1",
                    &[&row.get::<_, String>("credential_revision")],
                )
                .await?
                .get(0);
            if stored != input.credential {
                return Err(StoreError::IdempotencyConflict);
            }
            let result = operation(&row)?;
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let id = if input.id.is_empty() {
            new_id()
        } else {
            input.id.to_owned()
        };
        let credential = new_id();
        let op_id = new_id();
        if input.id.is_empty() {
            if input.expected != 0 {
                return Err(StoreError::ConfigurationRevisionConflict);
            }
            tx.execute("INSERT INTO workflow_connections (id,display_name,provider,capability,revision,enabled,app_id,candidate_credential_revision,status,settings) VALUES ($1,$2,$3,$4,1,true,'',$5,'pending',$6)", &[&id,&input.name,&input.provider,&input.capability,&credential,&settings]).await?;
        } else {
            let current = locked(&tx, &id).await?;
            if current.provider != input.provider
                || current.capability != input.capability
                || current.settings != input.settings
            {
                return Err(StoreError::InvalidInput(
                    "Provider and destination are immutable",
                ));
            }
            if current.revision != input.expected {
                return Err(StoreError::ConfigurationRevisionConflict);
            }
            tx.execute("UPDATE workflow_connections SET revision=revision+1,display_name=$2,settings=$3,candidate_credential_revision=$4,safe_reason='',status=CASE WHEN enabled THEN 'pending' ELSE 'disabled' END WHERE id=$1", &[&id,&input.name,&settings,&credential]).await?;
        }
        tx.execute("INSERT INTO workflow_connection_credentials (revision_id,connection_id,value) VALUES ($1,$2,$3)", &[&credential,&id,&input.credential]).await?;
        let value = locked(&tx, &id).await?;
        tx.execute("INSERT INTO workflow_connection_operations (id,idempotency_key,connection_id,kind,safe_request,credential_revision,connection_revision,state,result) VALUES ($1,$2,$3,'save',$4,$5,$6,'accepted',$7)", &[&op_id,&input.key,&id,&safe,&credential,&revision(value.revision)?,&safe_json(&value)?]).await?;
        let result = operation(
            &tx.query_one(
                "SELECT * FROM workflow_connection_operations WHERE id=$1",
                &[&op_id],
            )
            .await?,
        )?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }

    /// Load only the immutable candidate owned by a provider validation operation.
    /// # Errors
    /// Reject invalid operation identities and unsupported capabilities.
    pub async fn provider_candidate(&self, id: &str) -> Result<ProviderCandidate, StoreError> {
        validate_id(id, "operation_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client
            .query_opt(
                "SELECT * FROM workflow_connection_operations WHERE id=$1 AND kind='save'",
                &[&id],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let operation = operation(&row)?;
        if !matches!(
            operation.connection.capability.as_str(),
            "llm" | "container_registry"
        ) {
            return Err(StoreError::InvalidInput("capability"));
        }
        let credential = client
            .query_one(
                "SELECT value FROM workflow_connection_credentials WHERE revision_id=$1",
                &[&row.get::<_, String>("credential_revision")],
            )
            .await?
            .get(0);
        driver.abort();
        Ok(ProviderCandidate {
            operation,
            credential,
        })
    }
}

impl WorkflowStore {
    /// Deliver only the frozen credential of an exact owned, accepted session command.
    /// # Errors
    /// Reject cancelled, terminal, stale or mismatched commands. This is not a public read projection.
    pub async fn session_credential(&self, command_bytes: Vec<u8>) -> Result<String, StoreError> {
        let request = super::super::provider_calls::AuthorityRequest {
            command_bytes,
            action: "poll".into(),
            ..Default::default()
        };
        let response = self.session_authority(&request).await?;
        if !response.allowed {
            return Err(StoreError::InvalidInput("credential_scope"));
        }
        let (_, _, command) =
            ayni_orchestrator_events::workflow::decode_command(&request.command_bytes)
                .map_err(|_| StoreError::InvalidInput("command"))?;
        let binding = command.binding.ok_or(StoreError::InvalidInput("binding"))?;
        let config = command
            .frozen_configuration
            .ok_or(StoreError::InvalidInput("configuration"))?;
        if config.provider != "openai"
            || config.connection_id.is_empty()
            || config.credential_revision.is_empty()
        {
            return Err(StoreError::InvalidInput("credential_binding"));
        }
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row=client.query_opt("SELECT c.value FROM workflow_connection_credentials c JOIN workflow_turn_inputs i ON i.credential_revision=c.revision_id JOIN workflow_agent_configurations a ON a.workspace_id=i.workspace_id AND a.revision_id=i.configuration_revision_id WHERE i.turn_id=$1 AND i.workspace_id=$2 AND c.connection_id=$3 AND c.revision_id=$4 AND a.connection_id=c.connection_id",&[&binding.turn_id,&binding.workspace_id,&config.connection_id,&config.credential_revision]).await?.ok_or(StoreError::NotFound)?;
        let value = row.get(0);
        driver.abort();
        Ok(value)
    }
}
