use super::{Ledger, LedgerError, StoreOutcome};
use ayni_orchestrator_events::workflow::{
    decode_decision,
    transport::{ConsumerRole, RejectedDelivery, rejected_delivery},
};
use rusqlite::{OptionalExtension as _, TransactionBehavior, params};

const MAX_RECORDS: i64 = 256;
const MAX_BYTES: i64 = 64 * 1024 * 1024;
const RETENTION_SECONDS: u64 = 8 * 24 * 60 * 60;

/// Safe aggregate diagnostics. Exact bytes and producer strings are never exposed.
#[derive(Clone, Debug, Default)]
pub struct QuarantineHealth {
    pub records: u64,
    pub bytes: u64,
    pub unsupported: u64,
    pub full: bool,
}

#[derive(Clone, Debug)]
pub struct QuarantineReplay {
    pub role: ConsumerRole,
    pub source: String,
    pub id: String,
    pub digest: String,
    pub exact_bytes: Vec<u8>,
    pub consumer_sequence: u64,
}

#[derive(Clone, Debug, Default)]
pub struct LedgerPressure {
    pub records: u64,
    pub bytes: u64,
    pub pending_reports: u64,
    pub unresolved_commands: u64,
    pub full: bool,
}

impl Ledger {
    /// Retained authority pressure includes reservations and quarantine.
    /// # Errors
    /// Returns storage errors or invalid retained counters.
    pub fn pressure(&self) -> Result<LedgerPressure, LedgerError> {
        let (records, bytes, pending, unresolved): (i64,i64,i64,i64)=self.connection.query_row(
            "SELECT (SELECT count(*)+COALESCE(sum(terminal_reserved_records),0) FROM commands)+(SELECT count(*) FROM reports)+(SELECT count(*) FROM transport_quarantine), \
            COALESCE((SELECT sum(COALESCE(length(event_bytes),0)+terminal_reserved_bytes) FROM commands),0)+ \
            COALESCE((SELECT sum(COALESCE(length(event_bytes),0)+reserved_bytes+COALESCE(length(decision_bytes),0)) FROM reports),0)+ \
            COALESCE((SELECT sum(COALESCE(length(exact_bytes),0)+length(event_source)+length(event_id)+length(event_digest)+128) FROM transport_quarantine),0), \
            (SELECT count(*) FROM reports WHERE decision IS NULL), \
            (SELECT count(*) FROM commands WHERE state IN ('claim_pending','accepted','terminal_pending'))",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
        let records = u64::try_from(records).map_err(|_| LedgerError::Invalid)?;
        let bytes = u64::try_from(bytes).map_err(|_| LedgerError::Invalid)?;
        Ok(LedgerPressure {
            records,
            bytes,
            pending_reports: u64::try_from(pending).map_err(|_| LedgerError::Invalid)?,
            unresolved_commands: u64::try_from(unresolved).map_err(|_| LedgerError::Invalid)?,
            full: records + 3 > self.limits.max_records
                || bytes + 256 * 1024 + 64 * 1024 > self.limits.max_exact_bytes,
        })
    }

    /// Retain rejected transport evidence atomically before the consumer ACK.
    /// # Errors
    /// At capacity or on storage failure no row commits and no ACK may be sent.
    pub fn quarantine_transport(
        &mut self,
        role: ConsumerRole,
        bytes: &[u8],
        consumer_sequence: u64,
        now_seconds: u64,
    ) -> Result<StoreOutcome, LedgerError> {
        if role == ConsumerRole::ApplicationReport || consumer_sequence == 0 {
            return Err(LedgerError::Invalid);
        }
        let event = rejected_delivery(role, bytes).ok_or(LedgerError::Invalid)?;
        self.retain_quarantine(role, bytes, consumer_sequence, now_seconds, &event)
    }

    /// Retain only identity/digest when an authenticated application decision
    /// cannot apply to this retained report. It never grants execution authority.
    ///
    /// # Errors
    /// Returns input, capacity, or storage errors; the caller must not ACK failures.
    pub fn quarantine_inapplicable_decision(
        &mut self,
        bytes: &[u8],
        consumer_sequence: u64,
        now_seconds: u64,
    ) -> Result<StoreOutcome, LedgerError> {
        if consumer_sequence == 0 {
            return Err(LedgerError::Invalid);
        }
        let (identity, _, _) = decode_decision(bytes).map_err(|_| LedgerError::Invalid)?;
        let event = RejectedDelivery {
            source: identity.source,
            id: identity.id,
            digest: super::storage::exact_digest(bytes),
            unsupported_version: false,
        };
        self.retain_quarantine(
            ConsumerRole::ExecutorDecision,
            bytes,
            consumer_sequence,
            now_seconds,
            &event,
        )
    }

    fn retain_quarantine(
        &mut self,
        role: ConsumerRole,
        bytes: &[u8],
        consumer_sequence: u64,
        now_seconds: u64,
        event: &RejectedDelivery,
    ) -> Result<StoreOutcome, LedgerError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<String> = transaction.query_row("SELECT event_digest FROM transport_quarantine WHERE consumer_role=?1 AND event_source=?2 AND event_id=?3 AND event_digest=?4", params![role.as_str(),event.source,event.id,event.digest], |row| row.get(0)).optional()?;
        if prior.is_some() {
            transaction.commit()?;
            return Ok(StoreOutcome::Duplicate);
        }
        let conflict = transaction.query_row("SELECT event_digest FROM transport_quarantine WHERE consumer_role=?1 AND event_source=?2 AND event_id=?3 LIMIT 1", params![role.as_str(),event.source,event.id], |row|row.get::<_,String>(0)).optional()?.is_some();
        let exact = (event.unsupported_version && !conflict).then_some(bytes);
        let reason = if conflict {
            "event_conflict"
        } else if exact.is_some() {
            "unsupported_version"
        } else {
            "malformed"
        };
        let added = i64::try_from(
            exact.map_or(0, <[u8]>::len)
                + event.source.len()
                + event.id.len()
                + event.digest.len()
                + 128,
        )
        .map_err(|_| LedgerError::Capacity)?;
        let (records, used): (i64,i64) = transaction.query_row("SELECT count(*),COALESCE(sum(COALESCE(length(exact_bytes),0)+length(event_source)+length(event_id)+length(event_digest)+128),0) FROM transport_quarantine", [], |row|Ok((row.get(0)?,row.get(1)?)))?;
        if records >= MAX_RECORDS || used.saturating_add(added) > MAX_BYTES {
            return Err(LedgerError::QuarantineCapacity);
        }
        // Quarantine consumes the same global reserve as commands and reports.
        super::storage::reserve(
            &transaction,
            self.limits,
            1,
            0,
            usize::try_from(added).map_err(|_| LedgerError::Capacity)?,
            0,
        )?;
        transaction.execute("INSERT INTO transport_quarantine (consumer_role,event_source,event_id,event_digest,reason,exact_bytes,consumer_sequence,recorded_at_seconds) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", params![role.as_str(),event.source,event.id,event.digest,reason,exact,i64::try_from(consumer_sequence).map_err(|_|LedgerError::Invalid)?,i64::try_from(now_seconds).map_err(|_|LedgerError::Invalid)?])?;
        transaction.commit()?;
        Ok(StoreOutcome::Stored)
    }

    /// Restricted exact bytes for post-upgrade replay through normal handlers.
    /// # Errors
    /// Returns retained-state or storage failures.
    pub fn quarantine_replay_batch(&self) -> Result<Vec<QuarantineReplay>, LedgerError> {
        let mut statement=self.connection.prepare("SELECT consumer_role,event_source,event_id,event_digest,exact_bytes,consumer_sequence FROM transport_quarantine WHERE exact_bytes IS NOT NULL ORDER BY recorded_at_seconds LIMIT 64")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (role, source, id, digest, exact_bytes, sequence) = row?;
            events.push(QuarantineReplay {
                role: match role.as_str() {
                    "executor_command" => ConsumerRole::ExecutorCommand,
                    "executor_decision" => ConsumerRole::ExecutorDecision,
                    _ => return Err(LedgerError::Invalid),
                },
                source,
                id,
                digest,
                exact_bytes,
                consumer_sequence: u64::try_from(sequence).map_err(|_| LedgerError::Invalid)?,
            });
        }
        Ok(events)
    }

    /// Remove exact quarantine bytes only after normal processing is durable.
    /// # Errors
    /// Returns storage errors.
    pub fn resolve_quarantine(&mut self, event: &QuarantineReplay) -> Result<(), LedgerError> {
        self.connection.execute("DELETE FROM transport_quarantine WHERE consumer_role=?1 AND event_source=?2 AND event_id=?3 AND event_digest=?4 AND exact_bytes=?5",params![event.role.as_str(),event.source,event.id,event.digest,event.exact_bytes])?;
        Ok(())
    }

    /// Retain valid future versions forever, compact malformed diagnostics after eight days.
    /// # Errors
    /// Returns storage or invalid-clock errors.
    pub fn prune_transport_quarantine(&mut self, now_seconds: u64) -> Result<u64, LedgerError> {
        let Some(cutoff) = now_seconds.checked_sub(RETENTION_SECONDS) else {
            return Ok(0);
        };
        let removed=self.connection.execute("DELETE FROM transport_quarantine WHERE exact_bytes IS NULL AND recorded_at_seconds < ?1",params![i64::try_from(cutoff).map_err(|_|LedgerError::Invalid)?])?;
        u64::try_from(removed).map_err(|_| LedgerError::Invalid)
    }

    /// Safe aggregate readiness/pressure diagnostics.
    /// # Errors
    /// Returns storage or invalid-state errors.
    pub fn quarantine_health(&self) -> Result<QuarantineHealth, LedgerError> {
        let (records,bytes,unsupported):(i64,i64,i64)=self.connection.query_row("SELECT count(*),COALESCE(sum(COALESCE(length(exact_bytes),0)+length(event_source)+length(event_id)+length(event_digest)+128),0),count(exact_bytes) FROM transport_quarantine",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        Ok(QuarantineHealth {
            records: u64::try_from(records).map_err(|_| LedgerError::Invalid)?,
            bytes: u64::try_from(bytes).map_err(|_| LedgerError::Invalid)?,
            unsupported: u64::try_from(unsupported).map_err(|_| LedgerError::Invalid)?,
            full: records >= MAX_RECORDS || bytes + 262_144 + 512 > MAX_BYTES,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn malformed_quarantine_is_digest_only_deduplicated_and_bounded()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = TempDir::new()?;
        let path = directory.path().join("ledger.sqlite");
        let mut ledger = Ledger::open(
            &path,
            super::super::LedgerLimits::m1_default(),
            "fixture",
            &[1; 32],
        )?;
        assert_eq!(
            ledger.quarantine_transport(
                ConsumerRole::ExecutorCommand,
                b"private malformed bytes",
                1,
                10
            )?,
            StoreOutcome::Stored
        );
        assert_eq!(
            ledger.quarantine_transport(
                ConsumerRole::ExecutorCommand,
                b"private malformed bytes",
                2,
                10
            )?,
            StoreOutcome::Duplicate
        );
        assert!(ledger.quarantine_replay_batch()?.is_empty());
        for index in 1..256 {
            ledger.quarantine_transport(
                ConsumerRole::ExecutorCommand,
                format!("malformed-{index}").as_bytes(),
                index + 1,
                10,
            )?;
        }
        assert!(matches!(
            ledger.quarantine_transport(ConsumerRole::ExecutorCommand, b"overflow", 257, 10),
            Err(LedgerError::QuarantineCapacity)
        ));
        drop(ledger);
        let mut ledger = Ledger::open(
            &path,
            super::super::LedgerLimits::m1_default(),
            "fixture",
            &[1; 32],
        )?;
        assert_eq!(ledger.quarantine_health()?.records, 256);
        assert!(ledger.quarantine_health()?.full);
        assert_eq!(
            ledger.prune_transport_quarantine(RETENTION_SECONDS + 10)?,
            0
        );
        assert_eq!(
            ledger.prune_transport_quarantine(RETENTION_SECONDS + 11)?,
            256
        );
        Ok(())
    }

    #[test]
    fn sqlite_failure_does_not_commit_quarantine_or_drop_existing_rows()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = TempDir::new()?;
        let mut ledger = Ledger::open(
            directory.path().join("ledger.sqlite"),
            super::super::LedgerLimits::m1_default(),
            "fixture",
            &[1; 32],
        )?;
        ledger.quarantine_transport(ConsumerRole::ExecutorDecision, b"first", 1, 10)?;
        ledger.connection.execute_batch("CREATE TRIGGER quarantine_full BEFORE INSERT ON transport_quarantine BEGIN SELECT RAISE(ABORT,'full'); END;")?;
        assert!(
            ledger
                .quarantine_transport(ConsumerRole::ExecutorDecision, b"second", 2, 10)
                .is_err()
        );
        assert_eq!(ledger.quarantine_health()?.records, 1);
        Ok(())
    }
}
