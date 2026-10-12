//! Durable delivery receipts and a guarded wake of existing accepted cleanup.
use super::{CleanStore, CleanStoreError, Connection, OptionalExtension, params, unix_seconds};
use ayni_orchestrator_events::{EventIdentity, v1::VerificationDecisionRecorded};
use prost::Message;
use sha2::{Digest, Sha256};

pub(super) const SCHEMA: &str = "CREATE TABLE verification_consumer(\n delivery_key TEXT PRIMARY KEY CHECK(length(delivery_key) IN (64,66)),\n digest BLOB NOT NULL CHECK(length(digest)=32),\n outcome TEXT NOT NULL CHECK(outcome IN ('woken','unchanged','rejected')),\n processed_at INTEGER NOT NULL CHECK(processed_at>=0),\n journal_id TEXT REFERENCES verification_journal(id) ON DELETE SET NULL);\nCREATE INDEX verification_consumer_retention ON verification_consumer(processed_at,delivery_key);";

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection.execute_batch(&format!("BEGIN IMMEDIATE; {SCHEMA} UPDATE clean_schema SET version=15 WHERE component='control_plane'; COMMIT;"))
        .map_err(CleanStoreError::Database)
}

impl CleanStore {
    pub(crate) fn consume_verification(
        &mut self,
        bytes: &[u8],
    ) -> Result<&'static str, CleanStoreError> {
        self.verification_transaction(|store| {
            match ayni_orchestrator_events::decisions::decode(bytes) {
                Ok((identity, decision)) => store.consume_decision(&identity, &decision, bytes),
                Err(_) => store.reject_delivery(bytes),
            }
        })
    }

    fn consume_decision(
        &mut self,
        identity: &EventIdentity,
        decision: &VerificationDecisionRecorded,
        bytes: &[u8],
    ) -> Result<&'static str, CleanStoreError> {
        let key = crate::coordinators::outbox::message_id(identity);
        let digest = Sha256::digest(decision.encode_to_vec()).to_vec();
        if let Some(previous) = self.delivery_digest(&key)? {
            return if previous == digest {
                Ok("duplicate")
            } else {
                self.reject_delivery(bytes)
            };
        }
        // Neither a controller-shaped source nor broker sequence grants authority.
        let trusted: Option<(String, Vec<u8>)> = self.connection.query_row(
            "SELECT j.id,o.payload FROM verification_outbox o JOIN verification_journal j ON j.id=o.journal_id WHERE o.source=?1 AND o.event_id=?2",
            params![identity.source, identity.id], |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(CleanStoreError::Database)?;
        if trusted
            .as_ref()
            .and_then(|(_, p)| ayni_orchestrator_events::decisions::decode(p).ok())
            != Some((identity.clone(), decision.clone()))
        {
            return self.reject_delivery(bytes);
        }
        self.admit_delivery_capacity()?;
        let cursor = self.runtime_cursor(&decision.runtime_profile_id)?;
        let matches = cursor
            .preparation
            .as_ref()
            .is_some_and(|p| p.generation == decision.preparation_id)
            && cursor.pending.as_ref().is_some_and(|p| {
                p.name == decision.execution_id
                    && p.profile_revision == decision.runtime_profile_revision_id
                    && !p.failed
            })
            && self.verification_cleanup_committed(&decision.runtime_profile_id, &cursor)?;
        let outcome = if matches {
            // Only eligibility changes: preserve evidence, authority, failure budgets
            // and the existing cleanup intent. Never invoke operator retry admission.
            self.connection.execute(
                "UPDATE reconciliation_cursors SET value=json_set(value,'$.next_at',0),resource_version=resource_version+1,updated_at=?2 WHERE name=?1",
                params![format!("managed:{}",decision.runtime_profile_id), unix_seconds()?],
            ).map_err(CleanStoreError::Database)?;
            "woken"
        } else {
            "unchanged"
        };
        self.insert_delivery(
            &key,
            &digest,
            outcome,
            trusted.as_ref().map(|(id, _)| id.as_str()),
        )?;
        Ok(outcome)
    }

    fn delivery_digest(&self, key: &str) -> Result<Option<Vec<u8>>, CleanStoreError> {
        self.connection
            .query_row(
                "SELECT digest FROM verification_consumer WHERE delivery_key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)
    }

    fn reject_delivery(&self, bytes: &[u8]) -> Result<&'static str, CleanStoreError> {
        let digest = Sha256::digest(bytes).to_vec();
        let key = format!("r:{:x}", Sha256::digest(bytes));
        if self.delivery_digest(&key)?.is_none() {
            self.admit_delivery_capacity()?;
            self.insert_delivery(&key, &digest, "rejected", None)?;
        }
        Ok("rejected")
    }

    fn admit_delivery_capacity(&self) -> Result<(), CleanStoreError> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM verification_consumer", [], |r| {
                r.get(0)
            })
            .map_err(CleanStoreError::Database)?;
        if count >= super::verification_outbox::MAX_PENDING {
            Err(CleanStoreError::BoundExceeded)
        } else {
            Ok(())
        }
    }

    fn insert_delivery(
        &self,
        key: &str,
        digest: &[u8],
        outcome: &str,
        journal_id: Option<&str>,
    ) -> Result<(), CleanStoreError> {
        self.connection
            .execute(
                "INSERT INTO verification_consumer VALUES(?1,?2,?3,?4,?5)",
                params![key, digest, outcome, unix_seconds()?, journal_id],
            )
            .map_err(CleanStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn prune_verification_deliveries(&self) -> Result<(), CleanStoreError> {
        self.connection.execute("DELETE FROM verification_consumer WHERE delivery_key IN (SELECT d.delivery_key FROM verification_consumer d LEFT JOIN verification_journal j ON j.id=d.journal_id WHERE d.processed_at<?1 AND NOT EXISTS(SELECT 1 FROM reconciliation_cursors c WHERE c.name='managed:'||j.profile_id AND (NOT json_valid(c.value) OR json_extract(c.value,'$.preparation.generation')=j.preparation_id OR json_extract(c.value,'$.pending.name')=j.execution_id)) ORDER BY d.processed_at,d.delivery_key LIMIT 128)", [unix_seconds()?.saturating_sub(super::verification_outbox::HISTORY_SECONDS)]).map_err(CleanStoreError::Database)?;
        Ok(())
    }
}
