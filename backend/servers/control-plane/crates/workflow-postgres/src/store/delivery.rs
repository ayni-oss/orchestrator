use super::{StoreError, WorkflowStore};

#[derive(Debug)]
pub struct DecisionAcknowledgement {
    pub event_id: String,
    pub executor_instance_id: String,
    pub stream_sequence: u64,
    pub publication_age: std::time::Duration,
}

impl WorkflowStore {
    /// Retain broker occupancy as a conservative admission-capacity observation.
    ///
    /// # Errors
    /// Returns invalid input, unavailable database, or retained-state errors.
    pub async fn observe_broker_capacity(
        &self,
        lane: &str,
        messages: u64,
        bytes: u64,
    ) -> Result<(), StoreError> {
        if !matches!(lane, "command" | "report" | "decision") {
            return Err(StoreError::InvalidInput("broker_lane"));
        }
        let messages = i64::try_from(messages).map_err(|_| StoreError::CorruptState)?;
        let bytes = i64::try_from(bytes).map_err(|_| StoreError::CorruptState)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        client.execute("INSERT INTO workflow_broker_capacity(lane,retained_messages,retained_bytes) VALUES($1,$2,$3) \
            ON CONFLICT(lane) DO UPDATE SET retained_messages=$2,retained_bytes=$3,observed_at=clock_timestamp()", &[&lane,&messages,&bytes]).await?;
        driver.abort();
        Ok(())
    }

    /// Read bounded targeted handoffs whose broker storage sequence is known.
    ///
    /// # Errors
    /// Returns database or invalid retained-state errors.
    pub async fn decision_acknowledgements(
        &self,
    ) -> Result<Vec<DecisionAcknowledgement>, StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows = client
            .query(
                "SELECT id,executor_instance_id,stream_sequence,extract(epoch FROM clock_timestamp()-updated_at)::double precision FROM ( \
                 SELECT o.id,r.executor_instance_id,o.stream_sequence,o.updated_at FROM workflow_outbox o \
                 JOIN workflow_report_receipts r ON r.decision_event_id=o.id \
                 WHERE o.state='stored' AND o.stream_sequence>0 \
                 UNION ALL SELECT decision_event_id,executor_instance_id,stream_sequence,updated_at \
                   FROM workflow_transport_rejections WHERE state='stored' AND stream_sequence>0 \
                 ) deliveries ORDER BY updated_at LIMIT 128",
                &[],
            )
            .await?;
        let result = rows
            .into_iter()
            .map(|row| {
                Ok(DecisionAcknowledgement {
                    event_id: row.get(0),
                    executor_instance_id: row.get(1),
                    stream_sequence: u64::try_from(row.get::<_, i64>(2))
                        .map_err(|_| StoreError::CorruptState)?,
                    publication_age: std::time::Duration::try_from_secs_f64(row.get(3))
                        .map_err(|_| StoreError::CorruptState)?,
                })
            })
            .collect();
        driver.abort();
        result
    }

    /// Finish a targeted decision handoff only after the broker confirms the
    /// target consumer's contiguous ACK floor includes this exact publication.
    ///
    /// # Errors
    /// Returns database, input, or invalid retained-state errors.
    pub async fn acknowledge_decision_delivery(
        &self,
        event_id: &str,
        sequence: u64,
        floor: u64,
    ) -> Result<(), StoreError> {
        if sequence == 0 || floor < sequence {
            return Err(StoreError::InvalidInput("decision_ack_floor"));
        }
        let sequence = i64::try_from(sequence).map_err(|_| StoreError::CorruptState)?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        // A concurrent republish may have replaced the observation. Leave it for
        // the next reconciliation rather than release an unproven obligation.
        client
            .execute(
                "UPDATE workflow_outbox o SET state='transferred',updated_at=clock_timestamp() \
            WHERE id=$1 AND stream_sequence=$2 AND state='stored' \
              AND EXISTS (SELECT 1 FROM workflow_report_receipts r WHERE r.decision_event_id=o.id)",
                &[&event_id, &sequence],
            )
            .await?;
        client.execute("UPDATE workflow_transport_rejections SET state='transferred',updated_at=clock_timestamp() \
            WHERE decision_event_id=$1 AND stream_sequence=$2 AND state='stored'", &[&event_id,&sequence]).await?;
        driver.abort();
        Ok(())
    }
}

pub(super) async fn mark_rejection_publication(
    tx: &tokio_postgres::Transaction<'_>,
    id: &str,
    bytes: &[u8],
    sequence: i64,
    published_at: f64,
) -> Result<bool, StoreError> {
    let changed=tx.execute("UPDATE workflow_transport_rejections SET updated_at=CASE WHEN state='pending' OR stream_sequence<$3 THEN to_timestamp($4::double precision) ELSE updated_at END,state='stored',stream_sequence=GREATEST(stream_sequence,$3), \
        attempt_count=attempt_count+1,next_attempt_at=clock_timestamp()+interval '30 seconds' \
        WHERE decision_event_id=$1 AND decision_bytes=$2 AND state IN ('pending','stored')", &[&id,&bytes,&sequence,&published_at]).await?;
    if changed == 1 {
        return Ok(true);
    }
    let retained=tx.query_opt("SELECT decision_bytes=$2 FROM workflow_transport_rejections WHERE decision_event_id=$1 AND state='transferred'", &[&id,&bytes]).await?;
    Ok(retained.is_some_and(|row| row.get(0)))
}
