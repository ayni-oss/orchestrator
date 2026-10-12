//! Durable runtime-operation receipts and guarded cursor acceptance.
use super::{CleanStore, CleanStoreError, Connection, OptionalExtension, params, unix_seconds};
use crate::profiles::ayni::preparation_state::{RuntimeCursor, RuntimeCursorExt};
use ayni_orchestrator_events::{EventIdentity, operations, v1::RuntimeOperationResultAvailable};
use prost::Message;

pub(super) const TABLE: &str = "CREATE TABLE runtime_operation_inbox(source TEXT NOT NULL,event_id TEXT NOT NULL,execution_id TEXT NOT NULL,payload BLOB NOT NULL,received_at INTEGER NOT NULL,decision TEXT NOT NULL CHECK(decision IN ('pending','accepted','rejected')),accepted_source TEXT,accepted_id TEXT,accepted_payload BLOB,attempts INTEGER NOT NULL DEFAULT 0,next_attempt_at INTEGER NOT NULL DEFAULT 0,stored_at INTEGER,broker_sequence INTEGER,PRIMARY KEY(source,event_id),UNIQUE(execution_id));";
pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection.execute_batch(&format!("BEGIN IMMEDIATE;{TABLE} UPDATE clean_schema SET version=16 WHERE component='control_plane';COMMIT;")).map_err(CleanStoreError::Database)
}
impl CleanStore {
    pub(crate) fn receive_runtime_operation(
        &mut self,
        identity: &EventIdentity,
        payload: &RuntimeOperationResultAvailable,
    ) -> Result<(), CleanStoreError> {
        operations::encode(identity, payload).map_err(|_| CleanStoreError::CorruptState)?;
        let execution = &payload
            .binding
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .execution_id;
        let tx = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let old: Option<(String,String,Vec<u8>)> = tx.query_row("SELECT source,event_id,payload FROM runtime_operation_inbox WHERE (source=?1 AND event_id=?2) OR execution_id=?3", params![identity.source, identity.id, execution], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(CleanStoreError::Database)?;
        if let Some((source, id, bytes)) = old {
            return if source == identity.source
                && id == identity.id
                && RuntimeOperationResultAvailable::decode(bytes.as_slice())
                    .map_err(|_| CleanStoreError::CorruptState)?
                    == *payload
            {
                Ok(())
            } else {
                Err(CleanStoreError::IdempotencyConflict)
            };
        }
        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM runtime_operation_inbox", [], |r| {
                r.get(0)
            })
            .map_err(CleanStoreError::Database)?;
        if count >= 100_000 {
            return Err(CleanStoreError::BoundExceeded);
        }
        tx.execute(
            "INSERT INTO runtime_operation_inbox(source,event_id,execution_id,payload,received_at,decision) VALUES(?1,?2,?3,?4,?5,'pending')",
            params![
                identity.source,
                identity.id,
                execution,
                payload.encode_to_vec(),
                unix_seconds()?
            ],
        )
        .map_err(CleanStoreError::Database)?;
        tx.commit().map_err(CleanStoreError::Database)
    }
    pub(crate) fn runtime_operation_receipt(
        &self,
        execution: &str,
    ) -> Result<Option<(EventIdentity, RuntimeOperationResultAvailable)>, CleanStoreError> {
        let row: Option<(String,String,Vec<u8>,String)> = self.connection.query_row("SELECT source,event_id,payload,decision FROM runtime_operation_inbox WHERE execution_id=?1", [execution], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(CleanStoreError::Database)?;
        match row {
            Some((source, id, bytes, decision)) if decision != "rejected" => Ok(Some((
                EventIdentity { source, id },
                RuntimeOperationResultAvailable::decode(bytes.as_slice())
                    .map_err(|_| CleanStoreError::CorruptState)?,
            ))),
            Some(_) => Err(CleanStoreError::InvalidTransition),
            None => Ok(None),
        }
    }
    pub(crate) fn save_runtime_operation_cursor(
        &mut self,
        profile: &str,
        next: &RuntimeCursor,
    ) -> Result<(), CleanStoreError> {
        self.verification_transaction(|store| {
            let current = store.runtime_cursor(profile)?;
            let Some(g) = &next.preparation else { return store.save_runtime_cursor(profile, next); };
            for (execution, identity) in &g.operation_observations {
                if current.preparation.as_ref().is_some_and(|c| c.operation_observations.get(execution) == Some(identity)) { continue; }
                let payload = store.validate_operation_acceptance(profile, &current, next, execution, identity)?;
                store.admit_outbox_capacity()?;
                let revision: i64 = store.connection.query_row("SELECT resource_version+1 FROM reconciliation_cursors WHERE name=?1",[format!("managed:{profile}")],|r|r.get(0)).map_err(CleanStoreError::Database)?;
                let fact_identity = EventIdentity { source:"ayni://orchestrator/controllers/runtime".into(),id:uuid::Uuid::now_v7().to_string() };
                let fact = ayni_orchestrator_events::v1::RuntimeOperationDecisionRecorded { observation:Some(payload.clone()),observation_source:identity.source.clone(),observation_id:identity.id.clone(),aggregate_revision:u64::try_from(revision).map_err(|_|CleanStoreError::CorruptState)? };
                let bytes = ayni_orchestrator_events::operation_decisions::encode(&fact_identity,&fact).map_err(|_|CleanStoreError::CorruptState)?;
                let changed = store.connection.execute("UPDATE runtime_operation_inbox SET decision='accepted',accepted_source=?3,accepted_id=?4,accepted_payload=?5 WHERE source=?1 AND event_id=?2 AND decision='pending'",params![identity.source,identity.id,fact_identity.source,fact_identity.id,bytes]).map_err(CleanStoreError::Database)?;
                if changed != 1 { return Err(CleanStoreError::StaleVersion); }
            }
            store.save_runtime_cursor(profile, next)?;
            if next.status.get("error-class").map(String::as_str) == Some("execution")
                && let Some(reason) = next.status.get("reason-code")
            { store.runtime_failure(profile, reason)?; }
            Ok(())
        })
    }
    fn validate_operation_acceptance(
        &self,
        profile: &str,
        current: &RuntimeCursor,
        next: &RuntimeCursor,
        execution: &str,
        identity: &EventIdentity,
    ) -> Result<RuntimeOperationResultAvailable, CleanStoreError> {
        let g = next
            .preparation
            .as_ref()
            .ok_or(CleanStoreError::StaleVersion)?;
        let previous = current
            .preparation
            .as_ref()
            .ok_or(CleanStoreError::StaleVersion)?;
        let (received, payload) = self
            .runtime_operation_receipt(execution)?
            .ok_or(CleanStoreError::StaleVersion)?;
        let b = payload
            .binding
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        let runtime_profile = self.runtime_profile(profile)?;
        if previous.generation != g.generation {
            return Err(CleanStoreError::StaleVersion);
        }
        if b.preparation_id != previous.generation {
            return Err(CleanStoreError::StaleVersion);
        }
        if received != *identity
            || b.runtime_profile_id != profile
            || b.repository_connection_id != runtime_profile.repository_connection_id
            || runtime_profile.active_revision_id.as_deref()
                != Some(b.runtime_profile_revision_id.as_str())
            || b.source_revision != previous.source_revision
            || b.lock_digest != previous.lock_digest
            || b.policy_digest != previous.policy_digest
            || b.platform != previous.platform
            || previous
                .operation_reporting
                .as_ref()
                .is_none_or(|r| r.source != identity.source)
            || current.retry_exhausted()
            || !operation_owned(current, execution, payload.kind)
        {
            return Err(CleanStoreError::StaleVersion);
        }
        Ok(payload)
    }
    pub(super) fn validate_runtime_operations(&self) -> Result<(), CleanStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT source,event_id,execution_id,payload FROM runtime_operation_inbox")
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        while let Some(r) = rows.next().map_err(CleanStoreError::Database)? {
            let identity = EventIdentity {
                source: r.get(0).map_err(CleanStoreError::Database)?,
                id: r.get(1).map_err(CleanStoreError::Database)?,
            };
            let execution: String = r.get(2).map_err(CleanStoreError::Database)?;
            let bytes: Vec<u8> = r.get(3).map_err(CleanStoreError::Database)?;
            let payload = RuntimeOperationResultAvailable::decode(bytes.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            operations::encode(&identity, &payload).map_err(|_| CleanStoreError::CorruptState)?;
            if payload
                .binding
                .as_ref()
                .is_none_or(|b| b.execution_id != execution)
            {
                return Err(CleanStoreError::CorruptState);
            }
        }
        Ok(())
    }
}
fn operation_owned(cursor: &RuntimeCursor, execution: &str, kind: i32) -> bool {
    use ayni_orchestrator_events::v1::RuntimeOperationKind as K;
    match K::try_from(kind).ok() {
        Some(K::Discovery) => {
            cursor.builder.job.as_deref() == Some(execution)
                || cursor.pending.as_ref().is_some_and(|p| {
                    !p.failed && p.phase.discovering()
                        && ayni_orchestrator_kubernetes::execution::operation_name(
                            &format!("discover:{}", p.name),
                            u64::from(p.publish_attempts),
                        ) == execution
                })
        }
        Some(K::Trust) => cursor
            .trust
            .as_ref()
            .is_some_and(|t| t.job == execution && t.state == "pending"),
        Some(K::Build) => cursor
            .pending
            .as_ref()
            .is_some_and(|p| !p.failed && p.phase.building() && p.build_name.as_deref() == Some(execution)),
        Some(K::Publication) => cursor
            .pending
            .as_ref()
            .is_some_and(|p| !p.failed && p.phase == ayni_orchestrator_application::runtime_preparation::ImagePreparationPhase::Publishing && p.publish_name.as_deref() == Some(execution)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_orchestrator_events::v1::{
        RuntimeOperationBinding, RuntimeOperationEvidenceReference, RuntimeOperationKind,
    };
    #[test]
    fn receipt_survives_restart_and_conflicting_physical_results_are_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("operations.sqlite");
        let mut store = CleanStore::open(&path, 3)?;
        let identity = EventIdentity {
            source: "ayni://orchestrator/supervisors/test".into(),
            id: uuid::Uuid::now_v7().to_string(),
        };
        let digest = format!("sha256:{}", "a".repeat(64));
        let payload = RuntimeOperationResultAvailable {
            binding: Some(RuntimeOperationBinding {
                repository_connection_id: "repository".into(),
                runtime_profile_id: "profile".into(),
                runtime_profile_revision_id: "revision".into(),
                preparation_id: "generation".into(),
                execution_id: "discovery-one".into(),
                pod_uid: "pod-one".into(),
                request_digest: digest.clone(),
                source_revision: "a".repeat(40),
                lock_digest: digest.clone(),
                policy_digest: digest.clone(),
                platform: "linux/amd64".into(),
            }),
            kind: RuntimeOperationKind::Discovery as i32,
            evidence: Some(RuntimeOperationEvidenceReference {
                evidence_id: "runtime-operation-json".into(),
                sha256: digest,
                size_bytes: 100,
                contract: operations::CONTRACT.into(),
            }),
        };
        store.receive_runtime_operation(&identity, &payload)?;
        drop(store);
        let mut store = CleanStore::open(&path, 3)?;
        store.receive_runtime_operation(&identity, &payload)?;
        assert_eq!(
            store.runtime_operation_receipt("discovery-one")?,
            Some((identity.clone(), payload.clone()))
        );
        let mut conflict = payload.clone();
        conflict.binding.as_mut().ok_or("binding")?.pod_uid = "replacement-pod".into();
        assert!(matches!(
            store.receive_runtime_operation(&identity, &conflict),
            Err(CleanStoreError::IdempotencyConflict)
        ));
        let other = EventIdentity {
            id: uuid::Uuid::now_v7().to_string(),
            ..identity.clone()
        };
        assert!(matches!(
            store.receive_runtime_operation(&other, &payload),
            Err(CleanStoreError::IdempotencyConflict)
        ));
        let bytes = operations::encode(&identity, &payload)?;
        assert_eq!(operations::decode(&bytes)?, (identity, payload));
        Ok(())
    }
}

impl CleanStore {
    pub(crate) fn claim_runtime_publications(
        &mut self,
        now: i64,
        limit: usize,
    ) -> Result<Vec<super::verification_outbox::Publication>, CleanStoreError> {
        self.verification_transaction(|store| {
            let publications={
                let mut q=store.connection.prepare("SELECT accepted_source,accepted_id,accepted_payload FROM runtime_operation_inbox WHERE decision='accepted' AND stored_at IS NULL AND next_attempt_at<=?1 ORDER BY next_attempt_at,received_at LIMIT ?2").map_err(CleanStoreError::Database)?;
                q.query_map(params![now,i64::try_from(limit.min(8)).map_err(|_|CleanStoreError::CorruptState)?],|r|Ok(super::verification_outbox::Publication {identity:EventIdentity {source:r.get(0)?,id:r.get(1)?},bytes:r.get(2)?})).map_err(CleanStoreError::Database)?.collect::<Result<Vec<_>,_>>().map_err(CleanStoreError::Database)?
            };
            for p in &publications {
                store.connection.execute("UPDATE runtime_operation_inbox SET attempts=MIN(attempts+1,1000000),next_attempt_at=?3+MIN(300,1<<MIN(9,attempts+1)) WHERE accepted_source=?1 AND accepted_id=?2",params![p.identity.source,p.identity.id,now]).map_err(CleanStoreError::Database)?;
            }
            Ok(publications)
        })
    }
    pub(crate) fn record_runtime_publication(
        &mut self,
        p: &super::verification_outbox::Publication,
        sequence: Option<u64>,
    ) -> Result<(), CleanStoreError> {
        let sequence = sequence
            .map(i64::try_from)
            .transpose()
            .map_err(|_| CleanStoreError::CorruptState)?;
        if sequence.is_some_and(|s| s <= 0) {
            return Err(CleanStoreError::CorruptState);
        }
        let stored = sequence.map(|_| unix_seconds()).transpose()?;
        let changed=self.connection.execute("UPDATE runtime_operation_inbox SET stored_at=?4,broker_sequence=?5 WHERE accepted_source=?1 AND accepted_id=?2 AND accepted_payload=?3 AND stored_at IS NULL",params![p.identity.source,p.identity.id,p.bytes,stored,sequence]).map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        Ok(())
    }
}

impl CleanStore {
    pub(crate) fn prune_runtime_operation_history(
        &mut self,
        now: i64,
    ) -> Result<(), CleanStoreError> {
        self.connection.execute("DELETE FROM runtime_operation_inbox WHERE rowid IN (SELECT i.rowid FROM runtime_operation_inbox i WHERE i.decision='accepted' AND i.stored_at<?1 AND NOT EXISTS(SELECT 1 FROM reconciliation_cursors c WHERE NOT json_valid(c.value) OR json_extract(c.value,'$.preparation.generation')=json_extract(CAST(i.accepted_payload AS TEXT),'$.data.observation.binding.preparationId')) ORDER BY i.stored_at LIMIT 128)",[now.saturating_sub(super::verification_outbox::HISTORY_SECONDS)]).map_err(CleanStoreError::Database)?;
        Ok(())
    }
    pub(super) fn validate_runtime_operation_delivery(&self) -> Result<(), CleanStoreError> {
        let mut q=self.connection.prepare("SELECT source,event_id,payload,accepted_source,accepted_id,accepted_payload FROM runtime_operation_inbox WHERE decision='accepted'").map_err(CleanStoreError::Database)?;
        let mut rows = q.query([]).map_err(CleanStoreError::Database)?;
        while let Some(r) = rows.next().map_err(CleanStoreError::Database)? {
            let source: String = r.get(0).map_err(CleanStoreError::Database)?;
            let id: String = r.get(1).map_err(CleanStoreError::Database)?;
            let payload: Vec<u8> = r.get(2).map_err(CleanStoreError::Database)?;
            let accepted = EventIdentity {
                source: r.get(3).map_err(CleanStoreError::Database)?,
                id: r.get(4).map_err(CleanStoreError::Database)?,
            };
            let bytes: Vec<u8> = r.get(5).map_err(CleanStoreError::Database)?;
            let (identity, fact) = ayni_orchestrator_events::operation_decisions::decode(&bytes)
                .map_err(|_| CleanStoreError::CorruptState)?;
            if identity != accepted
                || fact.observation_source != source
                || fact.observation_id != id
                || fact.observation
                    != Some(
                        RuntimeOperationResultAvailable::decode(payload.as_slice())
                            .map_err(|_| CleanStoreError::CorruptState)?,
                    )
            {
                return Err(CleanStoreError::CorruptState);
            }
        }
        let invalid:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM runtime_operation_inbox WHERE attempts<0 OR attempts>1000000 OR next_attempt_at<0 OR (decision!='accepted' AND (accepted_source IS NOT NULL OR accepted_id IS NOT NULL OR accepted_payload IS NOT NULL OR stored_at IS NOT NULL OR broker_sequence IS NOT NULL)) OR (stored_at IS NULL AND broker_sequence IS NOT NULL) OR (stored_at IS NOT NULL AND (stored_at<0 OR broker_sequence IS NULL OR broker_sequence<=0)))",[],|r|r.get(0)).map_err(CleanStoreError::Database)?;
        if invalid {
            return Err(CleanStoreError::CorruptState);
        }
        Ok(())
    }
}
