//! Guarded acceptance and shadow decisions. Existing cursor cleanup is the only scheduler.
use super::{CleanStore, CleanStoreError, Connection, OptionalExtension, params, unix_seconds};
use crate::profiles::ayni::{
    preparation_state::{RuntimeCursor, RuntimeCursorExt},
    verification::Evidence,
};
use ayni_orchestrator_application::runtime_activation::ActivateRuntime;
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{VerificationDecisionRecorded, VerificationResultAvailable},
};
use prost::Message;

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection.execute_batch("BEGIN IMMEDIATE;
ALTER TABLE verification_inbox RENAME TO verification_inbox_v11;
DROP INDEX verification_inbox_pending;
CREATE TABLE verification_inbox(
 source TEXT NOT NULL,event_id TEXT NOT NULL,execution_id TEXT NOT NULL,pod_uid TEXT NOT NULL,
 payload BLOB NOT NULL,received_at INTEGER NOT NULL,
 decision TEXT NOT NULL CHECK(decision IN ('pending','shadow_accepted','rejected')),
 reason TEXT NOT NULL DEFAULT '',next_attempt_at INTEGER NOT NULL DEFAULT 0,attempts INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(source,event_id),UNIQUE(execution_id,pod_uid));
INSERT INTO verification_inbox(source,event_id,execution_id,pod_uid,payload,received_at,decision)
 SELECT source,event_id,execution_id,pod_uid,payload,received_at,decision FROM verification_inbox_v11;
DROP TABLE verification_inbox_v11;
CREATE INDEX verification_inbox_pending ON verification_inbox(decision,next_attempt_at,received_at,source,event_id);
CREATE TABLE verification_journal(
 id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,preparation_id TEXT NOT NULL,execution_id TEXT NOT NULL,
 report_sha256 TEXT NOT NULL,aggregate_revision INTEGER NOT NULL,payload BLOB NOT NULL,created_at INTEGER NOT NULL,
 UNIQUE(profile_id,preparation_id,execution_id));
CREATE TABLE verification_outbox(
 journal_id TEXT PRIMARY KEY REFERENCES verification_journal(id),source TEXT NOT NULL,event_id TEXT NOT NULL,
 payload BLOB NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,stored_at INTEGER,UNIQUE(source,event_id));
UPDATE clean_schema SET version=12 WHERE component='control_plane'; COMMIT;")
        .map_err(CleanStoreError::Database)
}

impl CleanStore {
    // Savepoint-backed resource mutations participate in this single outer transaction.
    pub(super) fn verification_transaction<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T, CleanStoreError>,
    ) -> Result<T, CleanStoreError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = operation(self);
        match result {
            Ok(value) => match self.connection.execute_batch("COMMIT") {
                Ok(()) => Ok(value),
                Err(error) => {
                    self.connection
                        .execute_batch("ROLLBACK")
                        .map_err(CleanStoreError::Database)?;
                    Err(CleanStoreError::Database(error))
                }
            },
            Err(error) => {
                self.connection
                    .execute_batch("ROLLBACK")
                    .map_err(CleanStoreError::Database)?;
                Err(error)
            }
        }
    }

    /// The retained reader remains authoritative. Acceptance precedes cleanup and
    /// commits its projection, cursor cleanup intent, journal and outbox together.
    pub(crate) fn commit_verification(
        &mut self,
        command: &ActivateRuntime,
        expected_cursor: &str,
        next: &RuntimeCursor,
    ) -> Result<(), CleanStoreError> {
        self.verification_transaction(|store| {
            let pending = next.pending.as_ref().ok_or(CleanStoreError::CorruptState)?;
            let preparation = next.preparation.as_ref().ok_or(CleanStoreError::CorruptState)?;
            let report = pending.verification.as_ref().ok_or(CleanStoreError::CorruptState)?;
            let report_digest = ayni_orchestrator_domain::digest::sha256(&serde_json::to_vec(report).map_err(|_| CleanStoreError::CorruptState)?);
            if store.verification_replay(command,next,&report_digest)? { return Ok(()); }
            store.admit_verification_command(command,expected_cursor,next)?;
            store.admit_verification_observation(&command.profile_id, next)?;
            for table in ["verification_journal", "verification_outbox"] {
                let count: i64 = store.connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).map_err(CleanStoreError::Database)?;
                if count >= 100_000 { return Err(CleanStoreError::BoundExceeded); }
            }
            ayni_orchestrator_application::runtime_activation::activate(store, command).map_err(super::runtime_activation::activation_error)?;
            store.save_runtime_cursor(&command.profile_id, next)?;
            let profile = store.runtime_profile(&command.profile_id)?;
            let decision = VerificationDecisionRecorded {
                repository_connection_id: profile.repository_connection_id,
                runtime_profile_id: command.profile_id.clone(),runtime_profile_revision_id: command.revision_id.clone(),
                preparation_id: preparation.generation.clone(),execution_id: pending.name.clone(),
                aggregate_revision: profile.resource_version,report_sha256: report_digest.clone(),
                observation_source: pending.observation.as_ref().map(|o| o.source.clone()),observation_id: pending.observation.as_ref().map(|o| o.id.clone()),
            };
            let identity = EventIdentity { source: "ayni://orchestrator/controllers/verification".into(),id: uuid::Uuid::now_v7().to_string() };
            let bytes = ayni_orchestrator_events::decisions::encode(&identity,&decision).map_err(|_| CleanStoreError::CorruptState)?;
            store.connection.execute("INSERT INTO verification_journal(id,profile_id,preparation_id,execution_id,report_sha256,aggregate_revision,payload,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![identity.id,command.profile_id,preparation.generation,pending.name,report_digest,i64::try_from(profile.resource_version).map_err(|_| CleanStoreError::CorruptState)?,decision.encode_to_vec(),unix_seconds()?]).map_err(CleanStoreError::Database)?;
            store.connection.execute("INSERT INTO verification_outbox(journal_id,source,event_id,payload) VALUES(?1,?2,?1,?3)",params![identity.id,identity.source,bytes]).map_err(CleanStoreError::Database)?;
            if let Some(observation) = &pending.observation { store.accept_verification_receipt(observation)?; }
            Ok(())
        })
    }

    pub(super) fn verification_replay(
        &self,
        command: &ActivateRuntime,
        next: &RuntimeCursor,
        report_digest: &str,
    ) -> Result<bool, CleanStoreError> {
        let pending = next.pending.as_ref().ok_or(CleanStoreError::CorruptState)?;
        let preparation = next
            .preparation
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        let existing: Option<Vec<u8>> = self.connection.query_row(
                "SELECT payload FROM verification_journal WHERE profile_id=?1 AND preparation_id=?2 AND execution_id=?3",
                params![command.profile_id,preparation.generation,pending.name], |row| row.get(0)).optional().map_err(CleanStoreError::Database)?;
        if let Some(existing) = existing {
            let decision = VerificationDecisionRecorded::decode(existing.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            let current = self.runtime_profile(&command.profile_id)?;
            if decision.report_sha256 != report_digest
                || decision.runtime_profile_revision_id != command.revision_id
                || current.active_revision_id.as_deref() != Some(command.revision_id.as_str())
                || current
                    .active_runtime_artifact_id
                    .as_deref()
                    .map(|id| self.runtime_artifact(id))
                    .transpose()?
                    .is_none_or(|a| a.resolved_digest != command.artifact.digest)
            {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn admit_verification_command(
        &self,
        command: &ActivateRuntime,
        expected_cursor: &str,
        next: &RuntimeCursor,
    ) -> Result<(), CleanStoreError> {
        let pending = next.pending.as_ref().ok_or(CleanStoreError::CorruptState)?;
        let preparation = next
            .preparation
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        let report = pending
            .verification
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        let current = self.runtime_cursor(&command.profile_id)?;
        if serde_json::to_string(&current).map_err(|_| CleanStoreError::CorruptState)?
            != expected_cursor
        {
            return Err(CleanStoreError::StaleVersion);
        }
        self.current_verification(&command.profile_id, &current, unix_seconds()?)?;
        let admitted = current
            .pending
            .as_ref()
            .ok_or(CleanStoreError::StaleVersion)?;
        let frozen = current
            .preparation
            .as_ref()
            .ok_or(CleanStoreError::StaleVersion)?;
        let mut admitted_next = admitted.clone();
        admitted_next.verification.clone_from(&pending.verification);
        admitted_next.phase = pending.phase;
        admitted_next.observation.clone_from(&pending.observation);
        if serde_json::to_vec(frozen).map_err(|_| CleanStoreError::CorruptState)?
            != serde_json::to_vec(preparation).map_err(|_| CleanStoreError::CorruptState)?
            || serde_json::to_vec(&admitted_next).map_err(|_| CleanStoreError::CorruptState)?
                != serde_json::to_vec(pending).map_err(|_| CleanStoreError::CorruptState)?
            || admitted
                .certificate
                .as_ref()
                .is_none_or(|c| c.key_id != report.runtime.certificate_key_id)
        {
            return Err(CleanStoreError::StaleVersion);
        }
        crate::profiles::ayni::evidence_policy::admit_runtime(
            report,
            &admitted.request.expected,
            &admitted.request.revision,
            &admitted.lock_digest,
        )
        .map_err(|_| CleanStoreError::InvalidTransition)?;
        if pending.resolved_digest.as_deref() != Some(command.artifact.digest.as_str())
            || preparation.source_revision != command.source_revision.as_str()
            || preparation.lock_digest != report.lock_digest
        {
            return Err(CleanStoreError::StaleVersion);
        }
        Ok(())
    }

    pub(crate) fn complete_verification_cleanup(
        &mut self,
        profile: &str,
        expected: &str,
        completed: &RuntimeCursor,
    ) -> Result<(), CleanStoreError> {
        self.verification_transaction(|store| {
            let current = store.runtime_cursor(profile)?;
            if serde_json::to_string(&current).map_err(|_| CleanStoreError::CorruptState)?
                != expected
            {
                return Err(CleanStoreError::StaleVersion);
            }
            if let Some(pending) = &current.pending
                && store
                    .runtime_profile(profile)?
                    .active_revision_id
                    .as_deref()
                    != Some(pending.profile_revision.as_str())
            {
                return Err(CleanStoreError::StaleVersion);
            }
            store.save_runtime_cursor(profile, completed)
        })
    }

    pub(super) fn current_verification(
        &self,
        profile_id: &str,
        cursor: &RuntimeCursor,
        time: i64,
    ) -> Result<(), CleanStoreError> {
        let pending = cursor
            .pending
            .as_ref()
            .ok_or(CleanStoreError::StaleVersion)?;
        let preparation = cursor
            .preparation
            .as_ref()
            .ok_or(CleanStoreError::StaleVersion)?;
        let profile = self.runtime_profile(profile_id)?;
        let repository = self.repository_connection(&profile.repository_connection_id)?;
        if matches!(
            super::v1::RepositoryConnectionStatus::try_from(repository.status).ok(),
            Some(
                super::v1::RepositoryConnectionStatus::Revoked
                    | super::v1::RepositoryConnectionStatus::Failed
            )
        ) {
            return Err(CleanStoreError::StaleVersion);
        }
        if profile.active_revision_id.as_deref() != Some(pending.profile_revision.as_str())
            || !matches!(pending.phase,ayni_orchestrator_application::runtime_preparation::ImagePreparationPhase::Verifying | ayni_orchestrator_application::runtime_preparation::ImagePreparationPhase::Activating)
            || pending.failed
            || cursor.retry_exhausted()
            || cursor.accounting.executions.contains_key(&pending.name)
            || preparation.source_revision != pending.request.revision
            || preparation.lock_digest != pending.lock_digest
            || preparation.platform != pending.request.expected.platform
        {
            return Err(CleanStoreError::StaleVersion);
        }
        if u64::try_from(time).map_err(|_| CleanStoreError::CorruptState)? > pending.started_at.saturating_add(ayni_orchestrator_application::runtime_reconciliation::VERIFICATION_OBSERVATION_TIMEOUT_SECONDS) {
            return Err(CleanStoreError::DeadlineExpired);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn decide_verification(
        &mut self,
        identity: &EventIdentity,
        payload: &VerificationResultAvailable,
        evidence: &Evidence,
    ) -> Result<(), CleanStoreError> {
        self.decide_verification_with_interval(identity, payload, evidence, 60)
    }

    pub(crate) fn decide_verification_with_interval(
        &mut self,
        identity: &EventIdentity,
        payload: &VerificationResultAvailable,
        evidence: &Evidence,
        interval: u64,
    ) -> Result<(), CleanStoreError> {
        self.verification_transaction(|store| {
            let (decision, retained): (String, Vec<u8>) = store.connection.query_row("SELECT decision,payload FROM verification_inbox WHERE source=?1 AND event_id=?2",params![identity.source,identity.id], |r| Ok((r.get(0)?,r.get(1)?))).map_err(CleanStoreError::Database)?;
            if VerificationResultAvailable::decode(retained.as_slice()).map_err(|_| CleanStoreError::CorruptState)? != *payload { return Err(CleanStoreError::IdempotencyConflict); }
            if decision != "pending" { return Ok(()); }
            let binding = payload.binding.as_ref().ok_or(CleanStoreError::CorruptState)?;
            let time = unix_seconds()?;
            let reason = match store.shadow_authority(payload, time) {
                Ok(cursor) => match evidence {
                    Evidence::Unavailable => {
                        store.connection.execute("UPDATE verification_inbox SET attempts=MIN(attempts+1,1000000),next_attempt_at=?3,reason='evidence_unavailable' WHERE source=?1 AND event_id=?2",params![identity.source,identity.id,time+30]).map_err(CleanStoreError::Database)?;
                        return Ok(());
                    },
                    Evidence::Invalid(reason) => Some(*reason),
                    Evidence::Failed => None,
                    Evidence::Report(report) => {
                        let pending = cursor.pending.as_ref().ok_or(CleanStoreError::CorruptState)?;
                        if pending.certificate.as_ref().is_none_or(|c| c.key_id != report.runtime.certificate_key_id) {
                            Some("certificate_verification_mismatch")
                        } else if crate::profiles::ayni::evidence_policy::admit_runtime(report,&pending.request.expected,&binding.source_revision,&binding.lock_digest).is_err() {
                            Some("runtime_result_mismatch")
                        } else { None }
                    },
                },
                Err(CleanStoreError::DeadlineExpired) => Some("verification_deadline_expired"),
                Err(CleanStoreError::NotFound { .. } | CleanStoreError::StaleVersion) => Some("verification_authority_stale"),
                Err(error) => return Err(error),
            };
            if reason.is_none() && store.process_authoritative_receipt(identity, payload, evidence, time, interval)? { return Ok(()); }
            store.connection.execute("UPDATE verification_inbox SET decision=?3,reason=?4,next_attempt_at=0 WHERE source=?1 AND event_id=?2",params![identity.source,identity.id,if reason.is_some() {"rejected"} else {"shadow_accepted"},reason.unwrap_or("shadow_only")]).map_err(CleanStoreError::Database)?;
            Ok(())
        })
    }

    pub(super) fn shadow_authority(
        &self,
        payload: &VerificationResultAvailable,
        time: i64,
    ) -> Result<RuntimeCursor, CleanStoreError> {
        let binding = payload
            .binding
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        let cursor = self.runtime_cursor(&binding.runtime_profile_id)?;
        self.current_verification(&binding.runtime_profile_id, &cursor, time)?;
        let pending = cursor
            .pending
            .as_ref()
            .ok_or(CleanStoreError::StaleVersion)?;
        let preparation = cursor
            .preparation
            .as_ref()
            .ok_or(CleanStoreError::StaleVersion)?;
        let profile = self.runtime_profile(&binding.runtime_profile_id)?;
        let request =
            serde_json::to_vec(&pending.request).map_err(|_| CleanStoreError::CorruptState)?;
        let revision_matches = pending.profile_revision == binding.runtime_profile_revision_id;
        if profile.repository_connection_id != binding.repository_connection_id
            || !revision_matches
            || pending.name != binding.execution_id
            || preparation.generation != binding.preparation_id
            || preparation.policy_digest != binding.policy_digest
            || pending.request.revision != binding.source_revision
            || pending.lock_digest != binding.lock_digest
            || preparation.platform != binding.platform
            || pending.resolved_digest.as_deref() != Some(binding.resolved_image_digest.as_str())
            || ayni_orchestrator_domain::digest::sha256(&request) != binding.request_digest
        {
            return Err(CleanStoreError::StaleVersion);
        }
        Ok(cursor)
    }

    #[allow(
        dead_code,
        reason = "bounded durable read for the later broker publisher"
    )]
    pub(crate) fn unpublished_verification_events(
        &self,
    ) -> Result<Vec<(EventIdentity, Vec<u8>)>, CleanStoreError> {
        let mut statement=self.connection.prepare("SELECT o.source,o.event_id,o.payload FROM verification_outbox o JOIN verification_journal j ON j.id=o.journal_id WHERE o.stored_at IS NULL ORDER BY j.created_at,j.id LIMIT 128").map_err(CleanStoreError::Database)?;
        statement
            .query_map([], |r| {
                Ok((
                    EventIdentity {
                        source: r.get(0)?,
                        id: r.get(1)?,
                    },
                    r.get(2)?,
                ))
            })
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)
    }

    pub(super) fn validate_verification_journal(&self) -> Result<(), CleanStoreError> {
        let mut statement = self.connection.prepare("SELECT j.id,j.profile_id,j.preparation_id,j.execution_id,j.report_sha256,j.aggregate_revision,j.payload,o.source,o.event_id,o.payload,o.attempts,o.stored_at,j.created_at FROM verification_journal j LEFT JOIN verification_outbox o ON o.journal_id=j.id").map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        let mut count = 0;
        while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
            count += 1;
            let id: String = row.get(0).map_err(CleanStoreError::Database)?;
            let payload: Vec<u8> = row.get(6).map_err(CleanStoreError::Database)?;
            let bytes: Option<Vec<u8>> = row.get(9).map_err(CleanStoreError::Database)?;
            let bytes = bytes.ok_or(CleanStoreError::CorruptState)?;
            if payload.len() > ayni_orchestrator_events::MAX_EVENT_BYTES || count > 100_000 {
                return Err(CleanStoreError::CorruptState);
            }
            let decision = VerificationDecisionRecorded::decode(payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            let (identity, outbox) = ayni_orchestrator_events::decisions::decode(&bytes)
                .map_err(|_| CleanStoreError::CorruptState)?;
            if decision != outbox
                || identity.id != id
                || identity.id != row.get::<_, String>(8).map_err(CleanStoreError::Database)?
                || identity.source != row.get::<_, String>(7).map_err(CleanStoreError::Database)?
                || !journal_indices_match(row, &decision, &self.connection)?
                || row.get::<_, i64>(10).map_err(CleanStoreError::Database)? < 0
                || row
                    .get::<_, Option<i64>>(11)
                    .map_err(CleanStoreError::Database)?
                    .is_some_and(|v| v < 0)
                || row.get::<_, i64>(12).map_err(CleanStoreError::Database)? < 0
            {
                return Err(CleanStoreError::CorruptState);
            }
        }
        Ok(())
    }
}

fn journal_indices_match(
    row: &rusqlite::Row<'_>,
    decision: &VerificationDecisionRecorded,
    connection: &Connection,
) -> Result<bool, CleanStoreError> {
    if !super::verification_cutover::valid_causation(connection, decision)? {
        return Ok(false);
    }
    Ok(!(decision.runtime_profile_id
        != row.get::<_, String>(1).map_err(CleanStoreError::Database)?
        || decision.preparation_id
            != row.get::<_, String>(2).map_err(CleanStoreError::Database)?
        || decision.execution_id != row.get::<_, String>(3).map_err(CleanStoreError::Database)?
        || decision.report_sha256 != row.get::<_, String>(4).map_err(CleanStoreError::Database)?
        || i64::try_from(decision.aggregate_revision).ok()
            != Some(row.get::<_, i64>(5).map_err(CleanStoreError::Database)?)))
}
