use super::{StoreError, WorkflowStore, support};
use crate::model::{AgentConfiguration, TurnInputs};
use tokio_postgres::{Row, Transaction};

const COLUMNS: &str = "workspace_id,revision_id,instructions,provider,model,max_input_tokens,max_output_tokens,max_total_tokens,idle_timeout_seconds";

pub(super) async fn initialize(tx: &Transaction<'_>, workspace: &str) -> Result<(), StoreError> {
    tx.execute("INSERT INTO workflow_agent_configurations (workspace_id,revision_id,instructions,provider,model,max_input_tokens,max_output_tokens,max_total_tokens,idle_timeout_seconds) VALUES ($1,$1,'','scripted','scripted',8192,2048,10240,300)", &[&workspace]).await?;
    tx.execute(
        "INSERT INTO workflow_agent_configuration_heads (workspace_id,revision_id) VALUES ($1,$1)",
        &[&workspace],
    )
    .await?;
    Ok(())
}

pub(super) async fn freeze_turn_inputs(
    tx: &Transaction<'_>,
    workspace: &str,
    turn: &str,
) -> Result<(), StoreError> {
    // Lock the same head as updates. Admission observes exactly one committed revision.
    let row = tx.query_one("SELECT revision_id FROM workflow_agent_configuration_heads WHERE workspace_id=$1 FOR SHARE", &[&workspace]).await?;
    let revision: String = row.get(0);
    tx.execute("INSERT INTO workflow_turn_inputs (turn_id,workspace_id,configuration_revision_id) VALUES ($1,$2,$3)", &[&turn,&workspace,&revision]).await?;
    Ok(())
}

pub(super) async fn execution_inputs(
    tx: &Transaction<'_>,
    turn: &str,
) -> Result<Option<ayni_orchestrator_events::v1::AgentExecutionConfiguration>, StoreError> {
    let query = format!(
        "SELECT {COLUMNS} FROM workflow_agent_configurations WHERE (workspace_id,revision_id)=(SELECT workspace_id,configuration_revision_id FROM workflow_turn_inputs WHERE turn_id=$1)"
    );
    tx.query_opt(&query, &[&turn])
        .await?
        .as_ref()
        .map(|row| {
            let value = configuration(row)?;
            Ok(ayni_orchestrator_events::v1::AgentExecutionConfiguration {
                revision_id: value.revision_id,
                instructions: value.instructions,
                provider: value.provider,
                model: value.model,
                max_input_tokens: value.max_input_tokens,
                max_output_tokens: value.max_output_tokens,
                max_total_tokens: value.max_total_tokens,
                idle_timeout_seconds: value.idle_timeout_seconds,
            })
        })
        .transpose()
}

fn configuration(row: &Row) -> Result<AgentConfiguration, StoreError> {
    Ok(AgentConfiguration {
        workspace_id: row.get(0),
        revision_id: row.get(1),
        instructions: row.get(2),
        provider: row.get(3),
        model: row.get(4),
        max_input_tokens: support::unsigned(row.get(5))?,
        max_output_tokens: support::unsigned(row.get(6))?,
        max_total_tokens: support::unsigned(row.get(7))?,
        idle_timeout_seconds: support::unsigned(row.get(8))?,
    })
}

fn validate_configuration(value: &AgentConfiguration) -> Result<(), StoreError> {
    support::validate_id(&value.workspace_id, "workspace_id")?;
    support::validate_text(&value.instructions, 16_384, true, "instructions")?;
    support::validate_name(&value.model, "model")?;
    if !matches!(value.provider.as_str(), "scripted" | "openai")
        || (value.provider == "scripted" && value.model != "scripted")
    {
        return Err(StoreError::InvalidInput("provider_model"));
    }
    if !(1..=1_000_000).contains(&value.max_input_tokens)
        || !(1..=100_000).contains(&value.max_output_tokens)
        || value.max_total_tokens < value.max_input_tokens + value.max_output_tokens
        || value.max_total_tokens > 1_100_000
        || !(30..=86_400).contains(&value.idle_timeout_seconds)
    {
        return Err(StoreError::InvalidInput("configuration_limits"));
    }
    Ok(())
}

impl WorkflowStore {
    /// Read the current workspace configuration, or an immutable revision of it.
    ///
    /// # Errors
    /// Returns invalid input, missing scope/revision, unavailable authority or corrupt data.
    pub async fn get_agent_configuration(
        &self,
        workspace: &str,
        revision: &str,
    ) -> Result<AgentConfiguration, StoreError> {
        support::validate_id(workspace, "workspace_id")?;
        if !revision.is_empty() {
            support::validate_id(revision, "revision_id")?;
        }
        let (client, driver) = crate::connect(&self.database_url).await?;
        let query = format!(
            "SELECT {COLUMNS} FROM workflow_agent_configurations WHERE workspace_id=$1 AND revision_id=COALESCE(NULLIF($2,''),(SELECT revision_id FROM workflow_agent_configuration_heads WHERE workspace_id=$1))"
        );
        let row = client
            .query_opt(&query, &[&workspace, &revision])
            .await?
            .ok_or(StoreError::NotFound)?;
        let result = configuration(&row)?;
        driver.abort();
        Ok(result)
    }

    /// Atomically replace a workspace's current configuration with an immutable snapshot.
    /// Replays return the original snapshot even after later edits.
    ///
    /// # Errors
    /// Returns invalid inputs, revision/idempotency conflict, missing scope or database errors.
    pub async fn update_agent_configuration(
        &self,
        key: &str,
        expected_revision: &str,
        proposed: &AgentConfiguration,
    ) -> Result<AgentConfiguration, StoreError> {
        support::validate_key(key)?;
        support::validate_id(expected_revision, "expected_revision_id")?;
        if !proposed.revision_id.is_empty() {
            return Err(StoreError::InvalidInput("revision_id_is_server_assigned"));
        }
        validate_configuration(proposed)?;
        let digest = support::request_digest(
            "UpdateAgentConfiguration",
            &[
                proposed.workspace_id.as_bytes(),
                expected_revision.as_bytes(),
                proposed.instructions.as_bytes(),
                proposed.provider.as_bytes(),
                proposed.model.as_bytes(),
                &proposed.max_input_tokens.to_be_bytes(),
                &proposed.max_output_tokens.to_be_bytes(),
                &proposed.max_total_tokens.to_be_bytes(),
                &proposed.idle_timeout_seconds.to_be_bytes(),
            ],
        );
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let head = tx.query_opt("SELECT revision_id FROM workflow_agent_configuration_heads WHERE workspace_id=$1 FOR UPDATE", &[&proposed.workspace_id]).await?.ok_or(StoreError::NotFound)?;
        let replay = tx.query_opt("SELECT request_digest,revision_id FROM workflow_configuration_receipts WHERE workspace_id=$1 AND idempotency_key=$2", &[&proposed.workspace_id,&key]).await?;
        let revision = if let Some(row) = replay {
            support::ensure_digest(&row.get::<_, Vec<u8>>(0), &digest)?;
            row.get::<_, String>(1)
        } else {
            if head.get::<_, String>(0) != expected_revision {
                return Err(StoreError::ConfigurationRevisionConflict);
            }
            let revision = support::new_id();
            let input = i64::try_from(proposed.max_input_tokens)
                .map_err(|_| StoreError::InvalidInput("max_input_tokens"))?;
            let output = i64::try_from(proposed.max_output_tokens)
                .map_err(|_| StoreError::InvalidInput("max_output_tokens"))?;
            let total = i64::try_from(proposed.max_total_tokens)
                .map_err(|_| StoreError::InvalidInput("max_total_tokens"))?;
            let idle = i64::try_from(proposed.idle_timeout_seconds)
                .map_err(|_| StoreError::InvalidInput("idle_timeout_seconds"))?;
            tx.execute("INSERT INTO workflow_agent_configurations (workspace_id,revision_id,instructions,provider,model,max_input_tokens,max_output_tokens,max_total_tokens,idle_timeout_seconds) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)", &[&proposed.workspace_id,&revision,&proposed.instructions,&proposed.provider,&proposed.model,&input,&output,&total,&idle]).await?;
            tx.execute("UPDATE workflow_agent_configuration_heads SET revision_id=$2 WHERE workspace_id=$1", &[&proposed.workspace_id,&revision]).await?;
            tx.execute("INSERT INTO workflow_configuration_receipts (workspace_id,idempotency_key,request_digest,revision_id) VALUES ($1,$2,$3,$4)", &[&proposed.workspace_id,&key,&&digest[..],&revision]).await?;
            revision
        };
        let query = format!(
            "SELECT {COLUMNS} FROM workflow_agent_configurations WHERE workspace_id=$1 AND revision_id=$2"
        );
        let result = configuration(
            &tx.query_one(&query, &[&proposed.workspace_id, &revision])
                .await?,
        )?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }

    /// Read the exact immutable inputs of a turn within its complete owner scope.
    ///
    /// # Errors
    /// Returns invalid input, missing scoped inputs or unavailable authority.
    pub async fn get_turn_inputs(
        &self,
        workspace: &str,
        project: &str,
        conversation: &str,
        turn: &str,
    ) -> Result<TurnInputs, StoreError> {
        support::validate_scope(workspace, project, conversation)?;
        support::validate_id(turn, "turn_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let query = format!(
            "SELECT {COLUMNS} FROM workflow_agent_configurations WHERE workspace_id=$1 AND revision_id=(SELECT inputs.configuration_revision_id FROM workflow_turn_inputs inputs JOIN workflow_turns turn ON turn.id=inputs.turn_id WHERE inputs.workspace_id=$1 AND turn.project_id=$2 AND turn.conversation_id=$3 AND turn.id=$4)"
        );
        let row = client
            .query_opt(&query, &[&workspace, &project, &conversation, &turn])
            .await?
            .ok_or(StoreError::NotFound)?;
        let result = TurnInputs {
            turn_id: turn.to_owned(),
            configuration: configuration(&row)?,
        };
        driver.abort();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_rejects_unbounded_or_implicit_provider_inputs() {
        let mut value = AgentConfiguration {
            workspace_id: support::new_id(),
            revision_id: String::new(),
            instructions: String::new(),
            provider: "scripted".into(),
            model: "scripted".into(),
            max_input_tokens: 8192,
            max_output_tokens: 2048,
            max_total_tokens: 10240,
            idle_timeout_seconds: 300,
        };
        assert!(validate_configuration(&value).is_ok());
        value.provider = "openai".into();
        value.model.clear();
        assert!(validate_configuration(&value).is_err());
        value.model = "operator-selected-model".into();
        assert!(validate_configuration(&value).is_ok());
        value.max_total_tokens = 1;
        assert!(validate_configuration(&value).is_err());
        value.max_total_tokens = 10240;
        value.instructions = "a".repeat(16385);
        assert!(validate_configuration(&value).is_err());
    }
}
