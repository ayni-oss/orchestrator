//! Publication bookkeeping never grants application acceptance authority.
use super::{CleanStore, CleanStoreError, Connection, params, unix_seconds};
use ayni_orchestrator_events::EventIdentity;

pub(crate) const MAX_PENDING: i64 = 100_000;
pub(crate) const HISTORY_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Publication {
    pub identity: EventIdentity,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OutboxStats {
    pub pending: i64,
    pub bytes: i64,
    pub oldest_at: i64,
    pub retained: i64,
}

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection.execute_batch("BEGIN IMMEDIATE;
ALTER TABLE verification_outbox ADD COLUMN next_attempt_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE verification_outbox ADD COLUMN broker_sequence INTEGER;
ALTER TABLE verification_outbox ADD COLUMN last_error TEXT NOT NULL DEFAULT '';
CREATE INDEX verification_outbox_pending ON verification_outbox(next_attempt_at,journal_id) WHERE stored_at IS NULL;
CREATE INDEX verification_outbox_stored ON verification_outbox(stored_at,journal_id) WHERE stored_at IS NOT NULL;
UPDATE clean_schema SET version=14 WHERE component='control_plane'; COMMIT;")
        .map_err(CleanStoreError::Database)
}

impl CleanStore {
    pub(super) fn admit_outbox_capacity(&self) -> Result<(), CleanStoreError> {
        let pending = self.outbox_stats()?.pending;
        if pending >= MAX_PENDING {
            Err(CleanStoreError::BoundExceeded)
        } else {
            Ok(())
        }
    }

    pub(crate) fn outbox_stats(&self) -> Result<OutboxStats, CleanStoreError> {
        let mut stats = self.connection.query_row(
            "SELECT COUNT(*), COALESCE(SUM(length(o.payload)),0), COALESCE(MIN(j.created_at),0),
             (SELECT COUNT(*) FROM verification_outbox WHERE stored_at IS NOT NULL)
             FROM verification_outbox o JOIN verification_journal j ON j.id=o.journal_id WHERE o.stored_at IS NULL",
            [], |r| Ok(OutboxStats {pending:r.get(0)?, bytes:r.get(1)?, oldest_at:r.get(2)?, retained:r.get(3)?}),
        ).map_err(CleanStoreError::Database)?;
        let runtime = self.connection.query_row("SELECT COUNT(*),COALESCE(SUM(length(accepted_payload)),0),COALESCE(MIN(received_at),0),(SELECT COUNT(*) FROM runtime_operation_inbox WHERE stored_at IS NOT NULL) FROM runtime_operation_inbox WHERE decision='accepted' AND stored_at IS NULL",[],|r| Ok(OutboxStats {pending:r.get(0)?,bytes:r.get(1)?,oldest_at:r.get(2)?,retained:r.get(3)?})).map_err(CleanStoreError::Database)?;
        stats.pending += runtime.pending;
        stats.bytes += runtime.bytes;
        stats.retained += runtime.retained;
        if runtime.oldest_at > 0 && (stats.oldest_at == 0 || runtime.oldest_at < stats.oldest_at) {
            stats.oldest_at = runtime.oldest_at;
        }
        Ok(stats)
    }

    // Persist retry scheduling before any network wait, including process crashes.
    pub(crate) fn claim_publications(
        &mut self,
        now: i64,
    ) -> Result<Vec<Publication>, CleanStoreError> {
        self.verification_transaction(|store| {
            let publications = {
                let mut query = store.connection.prepare(
                    "SELECT source,event_id,payload FROM verification_outbox WHERE stored_at IS NULL AND next_attempt_at<=?1 ORDER BY next_attempt_at,journal_id LIMIT 8",
                ).map_err(CleanStoreError::Database)?;
                query.query_map([now], |r| Ok(Publication { identity:EventIdentity { source:r.get(0)?, id:r.get(1)? }, bytes:r.get(2)? }))
                    .map_err(CleanStoreError::Database)?.collect::<Result<Vec<_>,_>>().map_err(CleanStoreError::Database)?
            };
            for publication in &publications {
                let attempts: u32 = store.connection.query_row("SELECT attempts FROM verification_outbox WHERE source=?1 AND event_id=?2", params![publication.identity.source,publication.identity.id], |r| r.get(0)).map_err(CleanStoreError::Database)?;
                let delay = (1_i64 << attempts.saturating_add(1).min(9)).min(300);
                store.connection.execute("UPDATE verification_outbox SET attempts=MIN(attempts+1,1000000),next_attempt_at=?3 WHERE source=?1 AND event_id=?2", params![publication.identity.source,publication.identity.id,now.saturating_add(delay)]).map_err(CleanStoreError::Database)?;
            }
            Ok(publications)
        })
    }

    pub(crate) fn record_publication(
        &mut self,
        publication: &Publication,
        sequence: Option<u64>,
    ) -> Result<(), CleanStoreError> {
        let stored = sequence.map(|_| unix_seconds()).transpose()?;
        let sequence = sequence
            .map(i64::try_from)
            .transpose()
            .map_err(|_| CleanStoreError::CorruptState)?;
        if sequence.is_some_and(|v| v <= 0) {
            return Err(CleanStoreError::CorruptState);
        }
        let changed = self
            .connection
            .execute(
                "UPDATE verification_outbox SET stored_at=?4,broker_sequence=?5,last_error=?6
             WHERE source=?1 AND event_id=?2 AND payload=?3 AND stored_at IS NULL",
                params![
                    publication.identity.source,
                    publication.identity.id,
                    publication.bytes,
                    stored,
                    sequence,
                    if stored.is_some() {
                        ""
                    } else {
                        "broker_unavailable"
                    }
                ],
            )
            .map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        Ok(())
    }

    pub(super) fn validate_outbox_delivery(&self) -> Result<(), CleanStoreError> {
        let invalid: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM verification_outbox WHERE next_attempt_at<0 OR attempts>1000000 OR
             last_error NOT IN ('','broker_unavailable') OR
             (stored_at IS NULL AND broker_sequence IS NOT NULL) OR
             (stored_at IS NOT NULL AND (broker_sequence IS NULL OR broker_sequence<=0 OR last_error!='')))",
            [], |r| r.get(0),
        ).map_err(CleanStoreError::Database)?;
        if invalid || self.outbox_stats()?.pending > MAX_PENDING {
            Err(CleanStoreError::CorruptState)
        } else {
            Ok(())
        }
    }

    pub(crate) fn prune_verification_history(&mut self) -> Result<(), CleanStoreError> {
        let now = unix_seconds()?;
        self.prune_verification_history_at(now)?;
        self.prune_runtime_operation_history(now)
    }

    pub(crate) fn prune_verification_history_at(
        &mut self,
        now: i64,
    ) -> Result<(), CleanStoreError> {
        self.verification_transaction(|store| {
            let ids = {
                let mut query = store.connection.prepare(
                    "SELECT j.id FROM verification_journal j JOIN verification_outbox o ON o.journal_id=j.id
                     WHERE o.stored_at IS NOT NULL AND o.stored_at<?1 AND NOT EXISTS(
                       SELECT 1 FROM reconciliation_cursors c WHERE c.name='managed:'||j.profile_id AND
                       (NOT json_valid(c.value) OR json_extract(c.value,'$.preparation.generation')=j.preparation_id
                        OR json_extract(c.value,'$.pending.name')=j.execution_id))
                     ORDER BY o.stored_at,j.id LIMIT 128",
                ).map_err(CleanStoreError::Database)?;
                query.query_map([now.saturating_sub(HISTORY_SECONDS)], |r| r.get::<_,String>(0))
                    .map_err(CleanStoreError::Database)?.collect::<Result<Vec<_>,_>>().map_err(CleanStoreError::Database)?
            };
            for id in ids {
                store.connection.execute("DELETE FROM verification_outbox WHERE journal_id=?1", [&id]).map_err(CleanStoreError::Database)?;
                store.connection.execute("DELETE FROM verification_journal WHERE id=?1", [&id]).map_err(CleanStoreError::Database)?;
            }
            Ok(())
        })
    }
}
