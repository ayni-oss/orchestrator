//! Receipt only: no runtime state transition or resource event is performed here.
use super::{CleanStore, CleanStoreError, Connection, OptionalExtension, params, unix_seconds};
use ayni_orchestrator_events::{EventIdentity, v1::VerificationResultAvailable};
use prost::Message;

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE verification_inbox(
 source TEXT NOT NULL,event_id TEXT NOT NULL,execution_id TEXT NOT NULL,pod_uid TEXT NOT NULL,
 payload BLOB NOT NULL,received_at INTEGER NOT NULL,decision TEXT NOT NULL CHECK(decision='pending'),
 PRIMARY KEY(source,event_id),UNIQUE(execution_id,pod_uid));
CREATE INDEX verification_inbox_pending ON verification_inbox(received_at,source,event_id);
UPDATE clean_schema SET version=11 WHERE component='control_plane'; COMMIT;")
        .map_err(CleanStoreError::Database)
}

impl CleanStore {
    pub(crate) fn receive_verification(
        &mut self,
        identity: &EventIdentity,
        payload: &VerificationResultAvailable,
    ) -> Result<(), CleanStoreError> {
        // Revalidate even when invoked through another trusted transport.
        ayni_orchestrator_events::encode(identity, payload)
            .map_err(|_| CleanStoreError::CorruptState)?;
        let binding = payload
            .binding
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let existing: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT payload FROM verification_inbox WHERE source=?1 AND event_id=?2",
                params![identity.source, identity.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        if let Some(existing) = existing {
            let previous = VerificationResultAvailable::decode(existing.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            return if previous == *payload {
                Ok(())
            } else {
                Err(CleanStoreError::IdempotencyConflict)
            };
        }
        let count: i64 = transaction
            .query_row("SELECT COUNT(*) FROM verification_inbox", [], |row| {
                row.get(0)
            })
            .map_err(CleanStoreError::Database)?;
        // Fail closed under pressure. Never evict an unprocessed receipt to acknowledge new work.
        if count >= 100_000 {
            return Err(CleanStoreError::BoundExceeded);
        }
        let physical: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM verification_inbox WHERE execution_id=?1 AND pod_uid=?2)",params![binding.execution_id,binding.pod_uid], |row| row.get(0)).map_err(CleanStoreError::Database)?;
        if physical {
            return Err(CleanStoreError::IdempotencyConflict);
        }
        transaction.execute("INSERT INTO verification_inbox(source,event_id,execution_id,pod_uid,payload,received_at,decision) VALUES(?1,?2,?3,?4,?5,?6,'pending')",
            params![identity.source,identity.id,binding.execution_id,binding.pod_uid,payload.encode_to_vec(),unix_seconds()?]).map_err(CleanStoreError::Database)?;
        transaction.commit().map_err(CleanStoreError::Database)
    }

    pub(super) fn validate_verification_inbox(&self) -> Result<(), CleanStoreError> {
        let mut statement = self.connection.prepare("SELECT source,event_id,execution_id,pod_uid,payload,received_at,decision,reason,next_attempt_at,attempts FROM verification_inbox").map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        let mut count = 0;
        while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
            count += 1;
            let identity = EventIdentity {
                source: row.get(0).map_err(CleanStoreError::Database)?,
                id: row.get(1).map_err(CleanStoreError::Database)?,
            };
            let execution: String = row.get(2).map_err(CleanStoreError::Database)?;
            let pod: String = row.get(3).map_err(CleanStoreError::Database)?;
            let bytes: Vec<u8> = row.get(4).map_err(CleanStoreError::Database)?;
            let received: i64 = row.get(5).map_err(CleanStoreError::Database)?;
            let decision: String = row.get(6).map_err(CleanStoreError::Database)?;
            let reason: String = row.get(7).map_err(CleanStoreError::Database)?;
            let retry: i64 = row.get(8).map_err(CleanStoreError::Database)?;
            let attempts: i64 = row.get(9).map_err(CleanStoreError::Database)?;
            if reason.len() > 64
                || !reason.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                || retry < 0
                || !(0..=1_000_000).contains(&attempts)
                || (decision != "pending" && (retry != 0 || reason.is_empty()))
            {
                return Err(CleanStoreError::CorruptState);
            }
            if count > 100_000
                || bytes.len() > ayni_orchestrator_events::MAX_EVENT_BYTES
                || received < 0
                || !matches!(
                    decision.as_str(),
                    "pending" | "shadow_accepted" | "rejected"
                )
            {
                return Err(CleanStoreError::CorruptState);
            }
            let payload = VerificationResultAvailable::decode(bytes.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            ayni_orchestrator_events::encode(&identity, &payload)
                .map_err(|_| CleanStoreError::CorruptState)?;
            let binding = payload
                .binding
                .as_ref()
                .ok_or(CleanStoreError::CorruptState)?;
            if binding.execution_id != execution || binding.pod_uid != pod {
                return Err(CleanStoreError::CorruptState);
            }
        }
        Ok(())
    }

    pub(crate) fn pending_verifications(
        &self,
    ) -> Result<Vec<(EventIdentity, VerificationResultAvailable)>, CleanStoreError> {
        let mut statement = self.connection.prepare("SELECT source,event_id,payload FROM verification_inbox WHERE decision='pending' AND next_attempt_at<=unixepoch() ORDER BY next_attempt_at,received_at,source,event_id LIMIT 128").map_err(CleanStoreError::Database)?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(CleanStoreError::Database)?
            .map(|row| {
                let (source, id, bytes) = row.map_err(CleanStoreError::Database)?;
                Ok((
                    EventIdentity { source, id },
                    VerificationResultAvailable::decode(bytes.as_slice())
                        .map_err(|_| CleanStoreError::CorruptState)?,
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_replays_after_restart_without_transition_or_conflicting_overwrite()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("receipt.sqlite");
        let (identity, payload) = ayni_orchestrator_events::decode(include_bytes!(
            "../../../../../../proto/testdata/verification/report.json"
        ))?;
        let mut store = CleanStore::open(&path, 3)?;
        let before = store.event_watermark()?;
        store.receive_verification(&identity, &payload)?;
        drop(store);
        let mut store = CleanStore::open(&path, 3)?;
        store.receive_verification(&identity, &payload)?;
        assert_eq!(
            store.pending_verifications()?,
            vec![(identity.clone(), payload.clone())]
        );
        assert_eq!(store.event_watermark()?, before);
        let mut conflicting = payload.clone();
        conflicting.binding.as_mut().ok_or("binding")?.policy_digest =
            format!("sha256:{}", "f".repeat(64));
        assert!(matches!(
            store.receive_verification(&identity, &conflicting),
            Err(CleanStoreError::IdempotencyConflict)
        ));
        let mut another = identity.clone();
        another.id = uuid::Uuid::now_v7().to_string();
        assert!(matches!(
            store.receive_verification(&another, &payload),
            Err(CleanStoreError::IdempotencyConflict)
        ));
        assert_eq!(store.pending_verifications()?, vec![(identity, payload)]);
        Ok(())
    }
}
