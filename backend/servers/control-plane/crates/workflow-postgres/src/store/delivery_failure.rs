//! Durable delivery failure observations preserve exact retry bytes and admission.
use super::{StoreError, WorkflowStore, acceptance::append_activity, support::validate_id};
use ayni_orchestrator_events::workflow::{COMMAND_SUBJECT, decode_command};
use tokio_postgres::Transaction;

impl WorkflowStore {
    /// Record one failed publication without changing its retained event identity.
    ///
    /// An eligible, unclaimed command exposes a queued delivery wait. Repeated
    /// failures keep one activity until its cause changes or publication recovers.
    /// Already transferred ownership never regresses on a delayed broker failure.
    ///
    /// # Errors
    /// Returns invalid input, missing/mismatched retained bytes, or database errors.
    pub async fn mark_outbox_failed(
        &self,
        id: &str,
        exact_bytes: &[u8],
        safe_reason: &'static str,
    ) -> Result<(), StoreError> {
        validate_id(id, "outbox_id")?;
        if !matches!(safe_reason, "broker_unavailable" | "broker_capacity") {
            return Err(StoreError::InvalidInput("delivery_failure_reason"));
        }
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        let domain = transaction
            .query_opt(
                "SELECT conversation_id,turn_id,subject,event_bytes FROM workflow_outbox WHERE id=$1",
                &[&id],
            )
            .await?;
        if let Some(domain) = domain {
            if domain.get::<_, Vec<u8>>(3) != exact_bytes {
                return Err(StoreError::CorruptState);
            }
            let conversation_id: String = domain.get(0);
            let turn_id: String = domain.get(1);
            let subject: String = domain.get(2);
            // Immutable identity lookup precedes the same lock order used by
            // admission and report acceptance: conversation, Turn, then outbox.
            transaction
                .query_one(
                    "SELECT id FROM workflow_conversations WHERE id=$1 FOR UPDATE",
                    &[&conversation_id],
                )
                .await?;
            let turn = transaction
                .query_one(
                    "SELECT execution_state,dispatch_state FROM workflow_turns WHERE id=$1 FOR UPDATE",
                    &[&turn_id],
                )
                .await?;
            let outbox = transaction
                .query_one(
                    "SELECT state,last_safe_reason FROM workflow_outbox WHERE id=$1 FOR UPDATE",
                    &[&id],
                )
                .await?;
            if outbox.get::<_, String>(0) != "transferred" {
                transaction
                    .execute(
                        "UPDATE workflow_outbox SET last_safe_reason=$2, \
                     attempt_count=LEAST(attempt_count,9223372036854775806)+1, \
                     next_attempt_at=clock_timestamp()+interval '250 milliseconds', \
                     updated_at=CASE WHEN state='pending' THEN clock_timestamp() ELSE updated_at END WHERE id=$1 AND state IN ('pending','stored')",
                        &[&id, &safe_reason],
                    )
                    .await?;
                if subject == COMMAND_SUBJECT {
                    command_failure_activity(
                        &transaction,
                        exact_bytes,
                        turn.get(0),
                        turn.get(1),
                        outbox.get(1),
                        safe_reason,
                    )
                    .await?;
                }
            }
        } else {
            detached_failure(&transaction, id, exact_bytes).await?;
        }
        transaction.commit().await?;
        driver.abort();
        Ok(())
    }

    /// Bound expired history-token storage without removing authoritative history.
    ///
    /// # Errors
    /// Returns database errors. Only expired token rows are removed.
    pub async fn cleanup_history_snapshots(&self) -> Result<u64, StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        let count = client
            .execute(
                "DELETE FROM workflow_history_snapshots WHERE id IN \
                 (SELECT id FROM workflow_history_snapshots WHERE expires_at<clock_timestamp() \
                  ORDER BY expires_at LIMIT 1000)",
                &[],
            )
            .await?;
        driver.abort();
        Ok(count)
    }
}

async fn command_failure_activity(
    transaction: &Transaction<'_>,
    bytes: &[u8],
    execution: &str,
    dispatch: &str,
    prior_reason: &str,
    reason: &str,
) -> Result<(), StoreError> {
    if execution != "not_started"
        || !matches!(dispatch, "pending" | "stored" | "degraded")
        || (dispatch == "degraded" && prior_reason == reason)
    {
        return Ok(());
    }
    let (_, _, command) = decode_command(bytes).map_err(|_| StoreError::CorruptState)?;
    let binding = command.binding.ok_or(StoreError::CorruptState)?;
    transaction.execute(
        "UPDATE workflow_turns SET dispatch_state='degraded',updated_at=clock_timestamp() \
         WHERE id=$1 AND execution_state='not_started' AND dispatch_state IN ('pending','stored','degraded')",
        &[&binding.turn_id]).await?;
    append_activity(
        transaction,
        &binding,
        "dispatch",
        "queued",
        "delivery",
        reason,
    )
    .await
}

async fn detached_failure(
    transaction: &Transaction<'_>,
    id: &str,
    bytes: &[u8],
) -> Result<(), StoreError> {
    let changed=transaction.execute(
        "UPDATE workflow_transport_rejections SET attempt_count=LEAST(attempt_count,9223372036854775806)+1, \
         next_attempt_at=clock_timestamp()+interval '250 milliseconds',updated_at=CASE WHEN state='pending' THEN clock_timestamp() ELSE updated_at END \
         WHERE decision_event_id=$1 AND decision_bytes=$2 AND state IN ('pending','stored')",&[&id,&bytes]).await?;
    if changed == 0 {
        let retained=transaction.query_opt(
            "SELECT decision_bytes=$2 AND state='transferred' FROM workflow_transport_rejections WHERE decision_event_id=$1",
            &[&id,&bytes]).await?;
        if !retained.is_some_and(|row| row.get(0)) {
            return Err(StoreError::CorruptState);
        }
    }
    Ok(())
}

pub(super) async fn record_dispatch_recovery(
    transaction: &Transaction<'_>,
    bytes: &[u8],
) -> Result<(), StoreError> {
    let (_, _, command) = decode_command(bytes).map_err(|_| StoreError::CorruptState)?;
    let binding = command.binding.ok_or(StoreError::CorruptState)?;
    append_activity(transaction, &binding, "dispatch", "queued", "delivery", "").await
}
