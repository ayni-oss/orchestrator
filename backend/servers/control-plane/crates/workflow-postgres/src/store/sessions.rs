//! Conversation-scoped session authority. Generations fence physical workloads;
//! the interface contract remains v1.
use super::support::{new_id, request_digest, unsigned, validate_key, validate_scope};
use super::{StoreError, WorkflowStore};
use ayni_orchestrator_events::v1::SessionExecutionScope;
use tokio_postgres::{Row, Transaction};

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub generation: u64,
    pub workspace_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub state: String,
    pub idle_timeout_seconds: u64,
    /// Advisory readiness from one authoritative read snapshot; mutation rechecks.
    pub can_replace: bool,
}

fn session(row: &Row) -> Result<Session, StoreError> {
    Ok(Session {
        id: row.get(0),
        generation: unsigned(row.get(1))?,
        workspace_id: row.get(2),
        project_id: row.get(3),
        conversation_id: row.get(4),
        state: row.get(5),
        idle_timeout_seconds: unsigned(row.get(6))?,
        can_replace: false,
    })
}

impl WorkflowStore {
    /// Return retained generations for one conversation, newest first.
    ///
    /// # Errors
    /// Returns scope, database, or retained-state errors.
    pub async fn conversation_sessions(
        &self,
        workspace_id: &str,
        project_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<Session>, StoreError> {
        validate_scope(workspace_id, project_id, conversation_id)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        super::support::conversation(&tx, workspace_id, project_id, conversation_id).await?;
        let rows = tx.query("SELECT s.id,s.generation,s.workspace_id,s.project_id,s.conversation_id,s.state,s.idle_timeout_seconds, s.state IN ('idle','expired') AND NOT EXISTS (SELECT 1 FROM workflow_turns t WHERE t.conversation_id=s.conversation_id AND t.execution_state NOT IN ('completed','failed')) AND NOT EXISTS (SELECT 1 FROM workflow_provider_calls p JOIN workflow_turns t ON t.id=p.turn_id WHERE t.conversation_id=s.conversation_id AND (p.state IN ('sending','uncertain') OR NOT p.reservation_settled)) AND NOT EXISTS (SELECT 1 FROM workflow_outbox o WHERE o.conversation_id=s.conversation_id AND o.state <> 'transferred') AS can_replace FROM workflow_sessions s WHERE s.workspace_id=$1 AND s.project_id=$2 AND s.conversation_id=$3 ORDER BY s.generation DESC LIMIT 100", &[&workspace_id,&project_id,&conversation_id]).await?;
        let result = rows
            .iter()
            .map(|row| {
                let mut value = session(row)?;
                value.can_replace = row.get(7);
                Ok(value)
            })
            .collect();
        tx.commit().await?;
        driver.abort();
        result
    }

    /// Expire idle generations only after all accepted work and report handoffs
    /// have completed. The conversation lock serializes expiry with admission.
    ///
    /// # Errors
    /// Returns database or retained-state errors.
    pub async fn expire_idle_sessions(&self) -> Result<Vec<Session>, StoreError> {
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let candidates = tx.query("SELECT c.id FROM workflow_conversations c JOIN workflow_sessions s ON s.conversation_id=c.id WHERE s.state='idle' AND s.last_active_at + s.idle_timeout_seconds * interval '1 second' < clock_timestamp() ORDER BY c.id LIMIT 100 FOR UPDATE OF c SKIP LOCKED", &[]).await?;
        let mut expired = Vec::new();
        for row in candidates {
            let conversation_id: String = row.get(0);
            let rows = tx.query("UPDATE workflow_sessions s SET state='expired' WHERE conversation_id=$1 AND state='idle' AND last_active_at + idle_timeout_seconds * interval '1 second' < clock_timestamp() AND NOT EXISTS (SELECT 1 FROM workflow_turns t WHERE t.conversation_id=$1 AND t.execution_state NOT IN ('completed','failed')) AND NOT EXISTS (SELECT 1 FROM workflow_provider_calls p JOIN workflow_turns t ON t.id=p.turn_id WHERE t.conversation_id=$1 AND (p.state IN ('sending','uncertain') OR NOT p.reservation_settled)) AND NOT EXISTS (SELECT 1 FROM workflow_outbox o WHERE o.conversation_id=$1 AND o.state NOT IN ('transferred')) RETURNING id,generation,workspace_id,project_id,conversation_id,state,idle_timeout_seconds", &[&conversation_id]).await?;
            for row in rows {
                expired.push(session(&row)?);
            }
        }
        tx.commit().await?;
        driver.abort();
        Ok(expired)
    }

    /// Fence an idle generation. Active work requires recovery rather than
    /// replacing an accepted execution with a fresh one.
    ///
    /// # Errors
    /// Returns scope, active-session, database, or retained-state errors.
    pub async fn replace_idle_session(
        &self,
        workspace_id: &str,
        project_id: &str,
        conversation_id: &str,
        expected_generation: u64,
        idempotency_key: &str,
    ) -> Result<Session, StoreError> {
        validate_key(idempotency_key)?;
        let digest = request_digest(
            "ReplaceConversationSession",
            &[
                workspace_id.as_bytes(),
                project_id.as_bytes(),
                conversation_id.as_bytes(),
                &expected_generation.to_be_bytes(),
            ],
        )
        .to_vec();
        validate_scope(workspace_id, project_id, conversation_id)?;
        let generation = i64::try_from(expected_generation)
            .map_err(|_| StoreError::InvalidInput("session_generation"))?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        tx.query_opt("SELECT id FROM workflow_conversations WHERE workspace_id=$1 AND project_id=$2 AND id=$3 FOR UPDATE", &[&workspace_id,&project_id,&conversation_id]).await?.ok_or(StoreError::NotFound)?;
        if let Some(row) = tx.query_opt("SELECT r.request_digest,s.id,s.generation,s.workspace_id,s.project_id,s.conversation_id,s.state,s.idle_timeout_seconds FROM workflow_session_receipts r JOIN workflow_sessions s ON s.id=r.session_id AND s.generation=r.generation WHERE r.conversation_id=$1 AND r.idempotency_key=$2", &[&conversation_id,&idempotency_key]).await? {
            let retained: Vec<u8> = row.get(0);
            if retained != digest { return Err(StoreError::IdempotencyConflict); }
            let result = Session { id:row.get(1),generation:unsigned(row.get(2))?,workspace_id:row.get(3),project_id:row.get(4),conversation_id:row.get(5),state:"replaced".into(),idle_timeout_seconds:unsigned(row.get(7))?,can_replace:false };
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let active: bool = tx.query_one("SELECT EXISTS (SELECT 1 FROM workflow_turns WHERE conversation_id=$1 AND execution_state NOT IN ('completed','failed')) OR EXISTS (SELECT 1 FROM workflow_provider_calls p JOIN workflow_turns t ON t.id=p.turn_id WHERE t.conversation_id=$1 AND (p.state IN ('sending','uncertain') OR NOT p.reservation_settled)) OR EXISTS (SELECT 1 FROM workflow_outbox WHERE conversation_id=$1 AND state <> 'transferred')", &[&conversation_id]).await?.get(0);
        if active {
            return Err(StoreError::InvalidInput("session_has_retained_work"));
        }
        let changed = tx.execute("UPDATE workflow_sessions SET state='replaced' WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND generation=$4 AND state IN ('idle','expired')", &[&workspace_id,&project_id,&conversation_id,&generation]).await?;
        if changed != 1 {
            return Err(StoreError::InvalidInput("session_generation"));
        }
        let row = tx.query_one("SELECT id,generation,workspace_id,project_id,conversation_id,state,idle_timeout_seconds FROM workflow_sessions WHERE conversation_id=$1 AND generation=$2", &[&conversation_id,&generation]).await?;
        let result = session(&row)?;
        tx.execute("INSERT INTO workflow_session_receipts(conversation_id,idempotency_key,request_digest,session_id,generation) VALUES($1,$2,$3,$4,$5)", &[&conversation_id,&idempotency_key,&digest,&result.id,&generation]).await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
}

pub(super) async fn bind_turn(
    tx: &Transaction<'_>,
    turn_id: &str,
) -> Result<Option<SessionExecutionScope>, StoreError> {
    let Some(config) = tx.query_opt("SELECT t.workspace_id,t.project_id,t.conversation_id,c.idle_timeout_seconds FROM workflow_turns t JOIN workflow_turn_inputs i ON i.turn_id=t.id JOIN workflow_agent_configurations c ON c.workspace_id=i.workspace_id AND c.revision_id=i.configuration_revision_id WHERE t.id=$1", &[&turn_id]).await? else { return Ok(None); };
    let workspace_id: String = config.get(0);
    let project_id: String = config.get(1);
    let conversation_id: String = config.get(2);
    let timeout: i64 = config.get(3);
    let current = tx.query_opt("SELECT id,generation FROM workflow_sessions WHERE conversation_id=$1 AND state IN ('provisioning','idle') FOR UPDATE", &[&conversation_id]).await?;
    let (id, generation) = if let Some(row) = current {
        (row.get::<_, String>(0), row.get::<_, i64>(1))
    } else {
        let last = tx.query_opt("SELECT id,generation FROM workflow_sessions WHERE conversation_id=$1 ORDER BY generation DESC LIMIT 1", &[&conversation_id]).await?;
        let (id, generation) = if let Some(row) = last {
            (
                row.get(0),
                row.get::<_, i64>(1)
                    .checked_add(1)
                    .ok_or(StoreError::CorruptState)?,
            )
        } else {
            (new_id(), 1)
        };
        tx.execute("INSERT INTO workflow_sessions (id,generation,workspace_id,project_id,conversation_id,state,idle_timeout_seconds) VALUES ($1,$2,$3,$4,$5,'provisioning',$6)", &[&id,&generation,&workspace_id,&project_id,&conversation_id,&timeout]).await?;
        (id, generation)
    };
    tx.execute("UPDATE workflow_sessions SET state='provisioning',last_active_at=clock_timestamp(),idle_timeout_seconds=$3 WHERE id=$1 AND generation=$2", &[&id,&generation,&timeout]).await?;
    tx.execute(
        "INSERT INTO workflow_turn_sessions(turn_id,session_id,generation) VALUES($1,$2,$3)",
        &[&turn_id, &id, &generation],
    )
    .await?;
    tx.execute(
        "UPDATE workflow_turns SET executor_generation=$2 WHERE id=$1",
        &[&turn_id, &generation],
    )
    .await?;
    Ok(Some(SessionExecutionScope {
        session_id: id,
        generation: unsigned(generation)?,
        idle_timeout_seconds: unsigned(timeout)?,
    }))
}

pub(super) async fn finish_turn(tx: &Transaction<'_>, turn_id: &str) -> Result<(), StoreError> {
    tx.execute("UPDATE workflow_sessions s SET state='idle',last_active_at=clock_timestamp() FROM workflow_turn_sessions t WHERE t.turn_id=$1 AND t.session_id=s.id AND t.generation=s.generation AND s.state='provisioning'", &[&turn_id]).await?;
    Ok(())
}

pub(super) async fn current_generation(
    tx: &Transaction<'_>,
    turn_id: &str,
) -> Result<bool, StoreError> {
    Ok(tx.query_one("SELECT NOT EXISTS (SELECT 1 FROM workflow_turn_sessions t JOIN workflow_sessions s ON s.id=t.session_id AND s.generation=t.generation WHERE t.turn_id=$1 AND s.state IN ('expired','replaced'))", &[&turn_id]).await?.get(0))
}
