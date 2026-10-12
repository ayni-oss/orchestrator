//! Frozen authority, guarded receipt acceptance and physical-failure accounting.
use super::{CleanStore, CleanStoreError, Connection, OptionalExtension, params, unix_seconds};
use crate::profiles::ayni::preparation_state::RuntimeCursor;
use ayni_orchestrator_events::v1::verification_result_available::Outcome;
use ayni_orchestrator_events::{EventIdentity, v1::VerificationResultAvailable};
use prost::Message;

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection.execute_batch("BEGIN IMMEDIATE;
ALTER TABLE verification_inbox RENAME TO verification_inbox_v12;
DROP INDEX verification_inbox_pending;
CREATE TABLE verification_inbox(
 source TEXT NOT NULL,event_id TEXT NOT NULL,execution_id TEXT NOT NULL,pod_uid TEXT NOT NULL,
 payload BLOB NOT NULL,received_at INTEGER NOT NULL,
 decision TEXT NOT NULL CHECK(decision IN ('pending','shadow_accepted','accepted','rejected')),
 reason TEXT NOT NULL DEFAULT '',next_attempt_at INTEGER NOT NULL DEFAULT 0,attempts INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(source,event_id),UNIQUE(execution_id,pod_uid));
INSERT INTO verification_inbox SELECT * FROM verification_inbox_v12;
DROP TABLE verification_inbox_v12;
CREATE INDEX verification_inbox_pending ON verification_inbox(decision,next_attempt_at,received_at,source,event_id);
UPDATE clean_schema SET version=13 WHERE component='control_plane'; COMMIT;")
    .map_err(CleanStoreError::Database)
}

impl CleanStore {
    pub(crate) fn verification_cleanup_committed(
        &self,
        profile: &str,
        cursor: &RuntimeCursor,
    ) -> Result<bool, CleanStoreError> {
        let Some(pending) = &cursor.pending else {
            return Ok(false);
        };
        let Some(report) = &pending.verification else {
            return Ok(false);
        };
        let digest = pending
            .resolved_digest
            .as_deref()
            .ok_or(CleanStoreError::CorruptState)?;
        let command = crate::profiles::ayni::runtime_activation::command(
            profile,
            &pending.profile_revision,
            &pending.image,
            digest,
            report,
        )
        .map_err(|_| CleanStoreError::CorruptState)?;
        let report_digest = ayni_orchestrator_domain::digest::sha256(
            &serde_json::to_vec(report).map_err(|_| CleanStoreError::CorruptState)?,
        );
        match self.verification_replay(&command, cursor, &report_digest) {
            Ok(value) => Ok(value),
            Err(CleanStoreError::IdempotencyConflict) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn verification_receipt(
        &self,
        execution: &str,
    ) -> Result<Option<(EventIdentity, VerificationResultAvailable)>, CleanStoreError> {
        let mut statement = self.connection.prepare("SELECT source,event_id,payload,decision FROM verification_inbox WHERE execution_id=?1 ORDER BY received_at,source,event_id LIMIT 2").map_err(CleanStoreError::Database)?;
        let mut rows = statement
            .query([execution])
            .map_err(CleanStoreError::Database)?;
        let Some(row) = rows.next().map_err(CleanStoreError::Database)? else {
            return Ok(None);
        };
        let identity = EventIdentity {
            source: row.get(0).map_err(CleanStoreError::Database)?,
            id: row.get(1).map_err(CleanStoreError::Database)?,
        };
        let bytes: Vec<u8> = row.get(2).map_err(CleanStoreError::Database)?;
        let decision: String = row.get(3).map_err(CleanStoreError::Database)?;
        if rows.next().map_err(CleanStoreError::Database)?.is_some() {
            return Err(CleanStoreError::InvalidTransition);
        }
        if decision == "accepted" {
            return Ok(None);
        }
        if decision != "pending" {
            return Err(CleanStoreError::InvalidTransition);
        }
        Ok(Some((
            identity,
            VerificationResultAvailable::decode(bytes.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?,
        )))
    }

    pub(super) fn admit_verification_observation(
        &self,
        profile: &str,
        next: &RuntimeCursor,
    ) -> Result<(), CleanStoreError> {
        let pending = next.pending.as_ref().ok_or(CleanStoreError::CorruptState)?;
        let authority = &next
            .preparation
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .verification_authority;
        if authority.events() != pending.observation.is_some() {
            return Err(CleanStoreError::InvalidTransition);
        }
        if let Some(observation) = &pending.observation {
            let (identity, payload) = self
                .verification_receipt(&pending.name)?
                .ok_or(CleanStoreError::StaleVersion)?;
            if identity != *observation
                || authority
                    .reporting()
                    .is_none_or(|r| r.source != identity.source)
            {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            let binding = payload
                .binding
                .as_ref()
                .ok_or(CleanStoreError::CorruptState)?;
            if binding.runtime_profile_id != profile {
                return Err(CleanStoreError::StaleVersion);
            }
            match (
                &payload.outcome,
                pending.failed,
                pending.verification.is_some(),
            ) {
                (Some(Outcome::ReportAvailable(_)), false, true) => {}
                (Some(Outcome::ExecutionFailed(failure)), true, false)
                    if pending.name == binding.execution_id =>
                {
                    let reason = crate::profiles::ayni::verification::failure_reason(failure.code)
                        .map_err(|_| CleanStoreError::CorruptState)?;
                    if next
                        .accounting
                        .executions
                        .get(&pending.name)
                        .map(String::as_str)
                        != Some(reason)
                    {
                        return Err(CleanStoreError::InvalidTransition);
                    }
                }
                _ => return Err(CleanStoreError::InvalidTransition),
            }
            self.shadow_authority(&payload, unix_seconds()?)?;
        }
        Ok(())
    }

    pub(super) fn accept_verification_receipt(
        &self,
        identity: &EventIdentity,
    ) -> Result<(), CleanStoreError> {
        let changed = self.connection.execute("UPDATE verification_inbox SET decision='accepted',reason='authoritative',next_attempt_at=0 WHERE source=?1 AND event_id=?2 AND decision='pending'", params![identity.source,identity.id]).map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        Ok(())
    }

    pub(crate) fn commit_verification_failure(
        &mut self,
        profile: &str,
        expected: &str,
        next: &RuntimeCursor,
        reason: &str,
    ) -> Result<(), CleanStoreError> {
        self.verification_transaction(|store| {
            store.commit_verification_failure_effects(profile, expected, next, reason)
        })
    }

    fn commit_verification_failure_effects(
        &mut self,
        profile: &str,
        expected: &str,
        next: &RuntimeCursor,
        reason: &str,
    ) -> Result<(), CleanStoreError> {
        let current = self.runtime_cursor(profile)?;
        if serde_json::to_string(&current).map_err(|_| CleanStoreError::CorruptState)? != expected {
            return Err(CleanStoreError::StaleVersion);
        }
        // Deadline itself can be the terminal physical outcome, never an activation.
        match self.current_verification(profile, &current, unix_seconds()?) {
            Ok(()) => {}
            Err(CleanStoreError::DeadlineExpired) if reason == "runtime_verification_timeout" => {}
            Err(error) => return Err(error),
        }
        let pending = next.pending.as_ref().ok_or(CleanStoreError::CorruptState)?;
        if pending.observation.is_some() {
            self.admit_verification_observation(profile, next)?;
        }
        if !pending.failed || !next.accounting.executions.contains_key(&pending.name) {
            return Err(CleanStoreError::CorruptState);
        }
        self.save_runtime_cursor(profile, next)?;
        self.runtime_failure(profile, reason)?;
        if let Some(identity) = &pending.observation {
            self.accept_verification_receipt(identity)?;
        }
        Ok(())
    }

    pub(super) fn process_authoritative_receipt(
        &mut self,
        identity: &EventIdentity,
        payload: &VerificationResultAvailable,
        evidence: &crate::profiles::ayni::verification::Evidence,
        time: i64,
        interval: u64,
    ) -> Result<bool, CleanStoreError> {
        use crate::profiles::ayni::preparation_state::RuntimeCursorExt;
        let profile = &payload
            .binding
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .runtime_profile_id;
        let mut cursor = self.runtime_cursor(profile)?;
        if !cursor
            .preparation
            .as_ref()
            .is_some_and(|p| p.verification_authority.events())
        {
            return Ok(false);
        }
        if cursor
            .preparation
            .as_ref()
            .and_then(|p| p.verification_authority.reporting())
            .is_none_or(|r| r.source != identity.source)
        {
            self.reject_verification_receipt(identity, "producer_source_mismatch")?;
            return Ok(true);
        }
        if matches!(
            evidence,
            crate::profiles::ayni::verification::Evidence::Failed
        ) {
            let Some(ayni_orchestrator_events::v1::verification_result_available::Outcome::ExecutionFailed(failure)) = &payload.outcome else { return Err(CleanStoreError::CorruptState); };
            let reason = crate::profiles::ayni::verification::failure_reason(failure.code)
                .map_err(|_| CleanStoreError::CorruptState)?;
            if matches!(
                reason,
                "runtime_request_invalid" | "runtime_reporting_failed"
            ) {
                self.record_verification_reporting_failure(
                    profile,
                    &mut cursor,
                    identity,
                    reason,
                    time,
                    interval,
                )?;
                return Ok(true);
            }
            let expected =
                serde_json::to_string(&cursor).map_err(|_| CleanStoreError::CorruptState)?;
            let pending = cursor
                .pending
                .as_mut()
                .ok_or(CleanStoreError::CorruptState)?;
            pending.failed = true;
            pending.observation = Some(identity.clone());
            cursor
                .accounting
                .executions
                .insert(pending.name.clone(), reason.into());
            cursor.failures = cursor.failures.saturating_add(1);
            cursor.status.insert("reason-code".into(), reason.into());
            cursor.status.insert("last-error".into(), reason.into());
            cursor
                .status
                .insert("error-class".into(), "execution".into());
            cursor.next_at = if cursor.retry_exhausted() {
                0
            } else {
                u64::try_from(time).map_err(|_| CleanStoreError::CorruptState)?
                    + ayni_orchestrator_application::runtime_reconciliation::retry_delay(
                        interval,
                        cursor.failures,
                    )
            };
            // Already inside the receipt-drain transaction; all failure effects
            // participate directly without opening another outer BEGIN.
            self.commit_verification_failure_effects(profile, &expected, &cursor, reason)?;
        } else {
            self.connection.execute("UPDATE verification_inbox SET next_attempt_at=?3,reason='awaiting_authoritative_reader' WHERE source=?1 AND event_id=?2", params![identity.source,identity.id,time+30]).map_err(CleanStoreError::Database)?;
        }
        Ok(true)
    }

    fn record_verification_reporting_failure(
        &mut self,
        profile: &str,
        cursor: &mut RuntimeCursor,
        identity: &EventIdentity,
        reason: &str,
        time: i64,
        interval: u64,
    ) -> Result<(), CleanStoreError> {
        use ayni_orchestrator_application::runtime_preparation::FailureClass;
        let attempts = cursor
            .accounting
            .operational
            .entry(FailureClass::Reporting)
            .or_default();
        *attempts = attempts.saturating_add(1);
        cursor.next_at = u64::try_from(time).map_err(|_| CleanStoreError::CorruptState)?
            + ayni_orchestrator_application::runtime_reconciliation::retry_delay(
                interval, *attempts,
            );
        cursor.status.insert("reason-code".into(), reason.into());
        cursor.status.insert("last-error".into(), reason.into());
        cursor
            .status
            .insert("error-class".into(), "reporting".into());
        self.save_runtime_cursor(profile, cursor)?;
        self.runtime_failure(profile, reason)?;
        self.accept_verification_receipt(identity)?;
        Ok(())
    }

    pub(crate) fn reject_verification_receipt(
        &mut self,
        identity: &EventIdentity,
        reason: &str,
    ) -> Result<(), CleanStoreError> {
        self.connection.execute("UPDATE verification_inbox SET decision='rejected',reason=?3,next_attempt_at=0 WHERE source=?1 AND event_id=?2 AND decision='pending'", params![identity.source,identity.id,reason]).map_err(CleanStoreError::Database)?;
        Ok(())
    }
}

pub(super) fn valid_causation(
    connection: &Connection,
    decision: &ayni_orchestrator_events::v1::VerificationDecisionRecorded,
) -> Result<bool, CleanStoreError> {
    let (Some(source), Some(id)) = (&decision.observation_source, &decision.observation_id) else {
        return Ok(true);
    };
    let payload: Option<Vec<u8>> = connection.query_row("SELECT payload FROM verification_inbox WHERE source=?1 AND event_id=?2 AND execution_id=?3 AND decision='accepted'", params![source,id,decision.execution_id], |r| r.get(0)).optional().map_err(CleanStoreError::Database)?;
    let Some(payload) = payload else {
        return Ok(false);
    };
    let payload = VerificationResultAvailable::decode(payload.as_slice())
        .map_err(|_| CleanStoreError::CorruptState)?;
    let binding = payload
        .binding
        .as_ref()
        .ok_or(CleanStoreError::CorruptState)?;
    Ok(
        binding.repository_connection_id == decision.repository_connection_id
            && binding.runtime_profile_id == decision.runtime_profile_id
            && binding.runtime_profile_revision_id == decision.runtime_profile_revision_id
            && binding.preparation_id == decision.preparation_id
            && matches!(payload.outcome, Some(Outcome::ReportAvailable(_))),
    )
}
