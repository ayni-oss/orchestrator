use super::{StoreError, WorkflowStore};
use ayni_orchestrator_events::workflow::transport::{ConsumerRole, rejected_delivery};

const QUARANTINE_LOCK: i64 = 4_702_199_126_064_922;
const MAX_RECORDS: i64 = 1_024;
const MAX_BYTES: i64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct TransportHealth {
    pub records: u64,
    pub bytes: u64,
    pub unsupported: u64,
    pub full: bool,
}

#[derive(Clone, Debug)]
pub struct QuarantinedEvent {
    pub source: String,
    pub id: String,
    pub digest: String,
    pub exact_bytes: Vec<u8>,
}

impl WorkflowStore {
    /// Durably retain rejected report evidence before acknowledging transport.
    /// # Errors
    /// Returns capacity, validation or database errors. On error the delivery must remain unacknowledged.
    pub async fn quarantine_transport(
        &self,
        role: ConsumerRole,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        if role != ConsumerRole::ApplicationReport {
            return Err(StoreError::InvalidInput("consumer_role"));
        }
        let rejected =
            rejected_delivery(role, bytes).ok_or(StoreError::InvalidInput("supported_event"))?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        transaction
            .query_one("SELECT pg_advisory_xact_lock($1)", &[&QUARANTINE_LOCK])
            .await?;
        let existing = transaction.query_opt(
            "SELECT event_digest FROM workflow_transport_quarantine WHERE consumer_role=$1 AND event_source=$2 AND event_id=$3 AND event_digest=$4",
            &[&role.as_str(), &rejected.source, &rejected.id, &rejected.digest]).await?;
        if existing.is_none() {
            let conflict = transaction.query_opt(
                "SELECT event_digest FROM workflow_transport_quarantine WHERE consumer_role=$1 AND event_source=$2 AND event_id=$3 LIMIT 1",
                &[&role.as_str(), &rejected.source, &rejected.id]).await?.is_some();
            let exact = (rejected.unsupported_version && !conflict).then_some(bytes);
            let reason = if conflict {
                "event_conflict"
            } else if exact.is_some() {
                "unsupported_version"
            } else {
                "malformed"
            };
            let added = i64::try_from(
                exact.map_or(0, <[u8]>::len)
                    + rejected.source.len()
                    + rejected.id.len()
                    + rejected.digest.len()
                    + 128,
            )
            .map_err(|_| StoreError::Capacity("application_report_quarantine"))?;
            let totals = transaction.query_one(
                "SELECT count(*), COALESCE(sum(COALESCE(octet_length(exact_bytes),0)+octet_length(event_source)+octet_length(event_id)+octet_length(event_digest)+128),0)::bigint FROM workflow_transport_quarantine", &[]).await?;
            if totals.get::<_, i64>(0) >= MAX_RECORDS
                || totals.get::<_, i64>(1).saturating_add(added) > MAX_BYTES
            {
                return Err(StoreError::Capacity("application_report_quarantine"));
            }
            transaction.execute(
                "INSERT INTO workflow_transport_quarantine (consumer_role,event_source,event_id,event_digest,reason,exact_bytes) VALUES ($1,$2,$3,$4,$5,$6)",
                &[&role.as_str(), &rejected.source, &rejected.id, &rejected.digest, &reason, &exact]).await?;
        }
        transaction.commit().await?;
        driver.abort();
        Ok(())
    }

    /// Restricted internal replay queue. Exact bytes never enter metrics or diagnostics.
    /// # Errors
    /// Returns database errors.
    pub async fn quarantine_replay_batch(&self) -> Result<Vec<QuarantinedEvent>, StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows = client.query("SELECT event_source,event_id,event_digest,exact_bytes FROM workflow_transport_quarantine WHERE exact_bytes IS NOT NULL ORDER BY recorded_at LIMIT 64", &[]).await?;
        driver.abort();
        Ok(rows
            .iter()
            .map(|row| QuarantinedEvent {
                source: row.get(0),
                id: row.get(1),
                digest: row.get(2),
                exact_bytes: row.get(3),
            })
            .collect())
    }

    /// Remove future-version evidence only after normal authoritative acceptance succeeds.
    /// # Errors
    /// Returns database errors.
    pub async fn resolve_quarantine(&self, event: &QuarantinedEvent) -> Result<(), StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        client.execute("DELETE FROM workflow_transport_quarantine WHERE consumer_role='application_report' AND event_source=$1 AND event_id=$2 AND event_digest=$3 AND exact_bytes=$4", &[&event.source,&event.id,&event.digest,&event.exact_bytes]).await?;
        driver.abort();
        Ok(())
    }

    /// Compact only malformed/conflicting digest diagnostics older than eight days.
    /// # Errors
    /// Returns database errors.
    pub async fn cleanup_transport_quarantine(&self) -> Result<u64, StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        let removed = client.execute("DELETE FROM workflow_transport_quarantine WHERE exact_bytes IS NULL AND recorded_at < clock_timestamp()-interval '8 days'", &[]).await?;
        driver.abort();
        Ok(removed)
    }

    /// Safe aggregate health without content or producer-controlled strings.
    /// # Errors
    /// Returns database errors.
    pub async fn transport_health(&self) -> Result<TransportHealth, StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row = client.query_one("SELECT count(*), COALESCE(sum(COALESCE(octet_length(exact_bytes),0)+octet_length(event_source)+octet_length(event_id)+octet_length(event_digest)+128),0)::bigint, count(*) FILTER (WHERE exact_bytes IS NOT NULL) FROM workflow_transport_quarantine", &[]).await?;
        driver.abort();
        let records: i64 = row.get(0);
        let bytes: i64 = row.get(1);
        let unsupported: i64 = row.get(2);
        Ok(TransportHealth {
            records: u64::try_from(records).map_err(|_| StoreError::CorruptState)?,
            bytes: u64::try_from(bytes).map_err(|_| StoreError::CorruptState)?,
            unsupported: u64::try_from(unsupported).map_err(|_| StoreError::CorruptState)?,
            full: records >= MAX_RECORDS || bytes + 128 * 1024 + 512 > MAX_BYTES,
        })
    }
}
