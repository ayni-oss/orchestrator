//! Frozen coding authority, durable receipt and transactional projection/journal/outbox.
use super::{CleanStore, CleanStoreError, Connection, OptionalExtension, params, unix_seconds, v1};
use ayni_orchestrator_events::{
    EventIdentity, coding, coding_decisions,
    v1::{CodingDecisionRecorded, CodingExecutionBinding, CodingReportAvailable, CodingReportKind},
};
use ayni_orchestrator_runtime::{
    coding_events::CodingAdmission, verification::ReportingConfiguration,
};
use prost::Message;
pub(super) const TABLES: &str = r"CREATE TABLE coding_executions(attempt_id TEXT PRIMARY KEY,admission TEXT,stopped INTEGER NOT NULL DEFAULT 0,last_sequence INTEGER NOT NULL DEFAULT 0,pod_uid TEXT,terminal INTEGER NOT NULL DEFAULT 0,request_digest TEXT NOT NULL DEFAULT '');
CREATE TABLE coding_inbox(source TEXT NOT NULL,event_id TEXT NOT NULL,attempt_id TEXT NOT NULL,generation TEXT NOT NULL,pod_uid TEXT NOT NULL,sequence INTEGER NOT NULL,kind INTEGER NOT NULL,payload BLOB NOT NULL,received_at INTEGER NOT NULL,decision TEXT NOT NULL DEFAULT 'pending',accepted_source TEXT,accepted_id TEXT,accepted_payload BLOB,attempts INTEGER NOT NULL DEFAULT 0,next_attempt_at INTEGER NOT NULL DEFAULT 0,stored_at INTEGER,broker_sequence INTEGER,PRIMARY KEY(source,event_id),UNIQUE(attempt_id,generation,pod_uid,sequence));
CREATE UNIQUE INDEX coding_terminal ON coding_inbox(attempt_id,generation,pod_uid) WHERE kind=2;
";
pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection.execute_batch(&format!("BEGIN IMMEDIATE;{TABLES} INSERT INTO coding_executions(attempt_id) SELECT id FROM development_attempts; UPDATE clean_schema SET version=17 WHERE component='control_plane'; COMMIT;")).map_err(CleanStoreError::Database)
}
impl CleanStore {
    pub(crate) fn coding_admission(
        &self,
        id: &str,
    ) -> Result<Option<CodingAdmission>, CleanStoreError> {
        let row: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT admission FROM coding_executions WHERE attempt_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        row.flatten()
            .map(|s| serde_json::from_str(&s).map_err(|_| CleanStoreError::CorruptState))
            .transpose()
    }
    pub(crate) fn select_coding_execution(
        &mut self,
        id: &str,
        execution: &str,
        configuration: Option<ReportingConfiguration>,
    ) -> Result<Option<CodingAdmission>, CleanStoreError> {
        self.verification_transaction(|store| {
            let row: Option<(Option<String>, bool)> = store.connection.query_row(
                "SELECT admission,stopped FROM coding_executions WHERE attempt_id=?1",
                [id], |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional().map_err(CleanStoreError::Database)?;
            let configuration = match row {
                Some((raw, false)) => return raw.map(|raw| {
                    serde_json::from_str(&raw).map_err(|_| CleanStoreError::CorruptState)
                }).transpose(),
                Some((raw, true)) => raw.map(|raw| {
                    serde_json::from_str::<CodingAdmission>(&raw)
                        .map(|admission| admission.configuration)
                        .map_err(|_| CleanStoreError::CorruptState)
                }).transpose()?,
                None => configuration,
            };
            let attempt = store.development_attempt(id)?;
            let admission = configuration.map(|configuration| CodingAdmission {
                configuration,
                binding: CodingExecutionBinding {
                    task_id: attempt.development_task_id.clone(),
                    attempt_id: id.into(),
                    generation: uuid::Uuid::now_v7().to_string(),
                    execution_id: execution.into(),
                    pod_uid: String::new(),
                    snapshot_digest: ayni_orchestrator_domain::digest::sha256(
                        &attempt.snapshot.as_ref().map(Message::encode_to_vec).unwrap_or_default(),
                    ),
                    request_digest: String::new(),
                },
            });
            let raw = admission.as_ref().map(serde_json::to_string)
                .transpose().map_err(|_| CleanStoreError::CorruptState)?;
            store.connection.execute(
                "INSERT INTO coding_executions(attempt_id,admission) VALUES(?1,?2) ON CONFLICT(attempt_id) DO UPDATE SET admission=excluded.admission,stopped=0,last_sequence=0,pod_uid=NULL,terminal=0,request_digest=''",
                params![id, raw],
            ).map_err(CleanStoreError::Database)?;
            Ok(admission)
        })
    }
    pub(crate) fn bind_coding_request(
        &mut self,
        id: &str,
        generation: &str,
        digest: &str,
    ) -> Result<(), CleanStoreError> {
        let old:String=self.connection.query_row("SELECT request_digest FROM coding_executions WHERE attempt_id=?1 AND json_extract(admission,'$.binding.generation')=?2",params![id,generation],|r|r.get(0)).map_err(CleanStoreError::Database)?;
        if !old.is_empty() && old != digest {
            return Err(CleanStoreError::IdempotencyConflict);
        }
        self.connection
            .execute(
                "UPDATE coding_executions SET request_digest=?2 WHERE attempt_id=?1",
                params![id, digest],
            )
            .map_err(CleanStoreError::Database)?;
        Ok(())
    }
    pub(crate) fn stop_coding_execution(&mut self, id: &str) -> Result<(), CleanStoreError> {
        self.connection
            .execute(
                "UPDATE coding_executions SET stopped=1 WHERE attempt_id=?1",
                [id],
            )
            .map_err(CleanStoreError::Database)?;
        Ok(())
    }
    pub(crate) fn receive_coding(
        &mut self,
        identity: &EventIdentity,
        payload: &CodingReportAvailable,
    ) -> Result<(), CleanStoreError> {
        coding::encode(identity, payload).map_err(|_| CleanStoreError::CorruptState)?;
        let b = payload
            .binding
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        self.verification_transaction(|store| {
            let sequence = i64::try_from(payload.sequence).map_err(|_| CleanStoreError::CorruptState)?;
            let old: Option<(String, String, Vec<u8>)> = store.connection.query_row(
                "SELECT source,event_id,payload FROM coding_inbox WHERE (source=?1 AND event_id=?2) OR (attempt_id=?3 AND generation=?4 AND pod_uid=?5 AND (sequence=?6 OR (kind=2 AND ?7=2)))",
                params![identity.source, identity.id, b.attempt_id, b.generation, b.pod_uid, sequence, payload.kind],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional().map_err(CleanStoreError::Database)?;
            if let Some((source, id, bytes)) = old {
                let previous = CodingReportAvailable::decode(bytes.as_slice())
                    .map_err(|_| CleanStoreError::CorruptState)?;
                return if source == identity.source && id == identity.id && previous == *payload {
                    Ok(())
                } else {
                    Err(CleanStoreError::IdempotencyConflict)
                };
            }
            let count: i64 = store.connection.query_row(
                "SELECT COUNT(*) FROM coding_inbox", [], |row| row.get(0),
            ).map_err(CleanStoreError::Database)?;
            if count >= 100_000 { return Err(CleanStoreError::BoundExceeded); }
            store.connection.execute(
                "INSERT INTO coding_inbox(source,event_id,attempt_id,generation,pod_uid,sequence,kind,payload,received_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![identity.source, identity.id, b.attempt_id, b.generation, b.pod_uid, sequence, payload.kind, payload.encode_to_vec(), unix_seconds()?],
            ).map_err(CleanStoreError::Database)?;
            Ok(())
        })
    }
    pub(crate) fn pending_coding(
        &self,
        id: &str,
    ) -> Result<Vec<(EventIdentity, CodingReportAvailable)>, CleanStoreError> {
        let mut q=self.connection.prepare("SELECT source,event_id,payload FROM coding_inbox WHERE attempt_id=?1 AND decision='pending' ORDER BY kind DESC,sequence DESC LIMIT 8").map_err(CleanStoreError::Database)?;
        q.query_map([id], |r| {
            Ok((
                EventIdentity {
                    source: r.get(0)?,
                    id: r.get(1)?,
                },
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(CleanStoreError::Database)?
        .map(|r| {
            let (i, b) = r.map_err(CleanStoreError::Database)?;
            Ok((
                i,
                CodingReportAvailable::decode(b.as_slice())
                    .map_err(|_| CleanStoreError::CorruptState)?,
            ))
        })
        .collect()
    }
    pub(crate) fn commit_coding_report(
        &mut self,
        identity: &EventIdentity,
        payload: &CodingReportAvailable,
        progress: Option<v1::WorkerProgress>,
        result: Option<v1::WorkerResult>,
        activity: Vec<v1::AgentActivity>,
        usage: v1::AttemptModelUsage,
    ) -> Result<bool, CleanStoreError> {
        if result.is_some() != (payload.kind == CodingReportKind::Final as i32) {
            return Err(CleanStoreError::CorruptState);
        }
        self.verification_transaction(|store| {
            let binding = payload.binding.as_ref().ok_or(CleanStoreError::CorruptState)?;
            let decision: String = store.connection.query_row(
                "SELECT decision FROM coding_inbox WHERE source=?1 AND event_id=?2 AND payload=?3",
                params![identity.source, identity.id, payload.encode_to_vec()],
                |row| row.get(0),
            ).map_err(CleanStoreError::Database)?;
            if decision != "pending" { return Ok(false); }
            let admission = store.coding_admission(&binding.attempt_id)?;
            let attempt = store.development_attempt(&binding.attempt_id)?;
            let task = store.development_task(&attempt.development_task_id)?;
            let (sequence, pod, terminal, stopped, request_digest): (i64, Option<String>, bool, bool, String) = store.connection.query_row(
                "SELECT last_sequence,pod_uid,terminal,stopped,request_digest FROM coding_executions WHERE attempt_id=?1",
                [&binding.attempt_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            ).map_err(CleanStoreError::Database)?;
            let current = admission.as_ref().is_some_and(|admission| {
                admission.configuration.source == identity.source
                    && admission.binding.generation == binding.generation
                    && admission.binding.snapshot_digest == binding.snapshot_digest
                    && admission.binding.execution_id == binding.execution_id
                    && admission.binding.task_id == binding.task_id
                    && request_digest == binding.request_digest
            });
            let active = current && pod.as_ref().is_none_or(|uid| uid == &binding.pod_uid)
                && !terminal && !stopped
                && task.active_attempt_id.as_deref() == Some(binding.attempt_id.as_str())
                && [v1::DevelopmentAttemptState::Queued as i32, v1::DevelopmentAttemptState::Running as i32].contains(&attempt.state)
                && attempt.snapshot.as_ref().and_then(|snapshot| snapshot.deadline_at.as_ref())
                    .is_some_and(|deadline| deadline.seconds > unix_seconds().unwrap_or(i64::MAX));
            let applied = active && (payload.kind == CodingReportKind::Final as i32
                || payload.sequence > u64::try_from(sequence).map_err(|_| CleanStoreError::CorruptState)?);
            store.admit_outbox_capacity()?;
            if applied {
                store.collect_worker(&binding.attempt_id, progress, result)?;
                if !activity.is_empty() { store.coding_activity(&binding.attempt_id, activity)?; }
                let fresh = store.development_attempt(&binding.attempt_id)?;
                store.coding_details(&binding.attempt_id, fresh.requested_input, usage)?;
                store.connection.execute(
                    "UPDATE coding_executions SET last_sequence=MAX(last_sequence,?2),pod_uid=?3,terminal=?4 WHERE attempt_id=?1",
                    params![binding.attempt_id, i64::try_from(payload.sequence).map_err(|_| CleanStoreError::CorruptState)?, binding.pod_uid, payload.kind == CodingReportKind::Final as i32],
                ).map_err(CleanStoreError::Database)?;
            }
            let revision = store.development_attempt(&binding.attempt_id)?.resource_version;
            let fact_identity = EventIdentity {
                source: "ayni://orchestrator/controllers/coding".into(),
                id: uuid::Uuid::now_v7().to_string(),
            };
            let fact = CodingDecisionRecorded {
                observation: Some(payload.clone()),
                observation_source: identity.source.clone(),
                observation_id: identity.id.clone(),
                aggregate_revision: revision,
                applied,
            };
            let bytes = coding_decisions::encode(&fact_identity, &fact).map_err(|_| CleanStoreError::CorruptState)?;
            store.connection.execute(
                "UPDATE coding_inbox SET decision='accepted',accepted_source=?3,accepted_id=?4,accepted_payload=?5 WHERE source=?1 AND event_id=?2",
                params![identity.source, identity.id, fact_identity.source, fact_identity.id, bytes],
            ).map_err(CleanStoreError::Database)?;
            Ok(applied)
        })
    }
    pub(crate) fn reject_coding_report(
        &mut self,
        identity: &EventIdentity,
    ) -> Result<(), CleanStoreError> {
        self.connection.execute("UPDATE coding_inbox SET decision='rejected' WHERE source=?1 AND event_id=?2 AND decision='pending'",params![identity.source,identity.id]).map_err(CleanStoreError::Database)?;
        Ok(())
    }
}
impl CleanStore {
    pub(crate) fn claim_coding_publications(
        &mut self,
        now: i64,
        limit: usize,
    ) -> Result<Vec<super::verification_outbox::Publication>, CleanStoreError> {
        self.verification_transaction(|store| {
            let publications = {
                let mut query = store.connection.prepare(
                    "SELECT accepted_source,accepted_id,accepted_payload FROM coding_inbox WHERE decision='accepted' AND stored_at IS NULL AND next_attempt_at<=?1 ORDER BY next_attempt_at,received_at LIMIT ?2",
                ).map_err(CleanStoreError::Database)?;
                let limit = i64::try_from(limit.min(8)).map_err(|_| CleanStoreError::CorruptState)?;
                query.query_map(params![now, limit], |row| {
                    Ok(super::verification_outbox::Publication {
                        identity: EventIdentity { source: row.get(0)?, id: row.get(1)? },
                        bytes: row.get(2)?,
                    })
                }).map_err(CleanStoreError::Database)?
                    .collect::<Result<Vec<_>, _>>().map_err(CleanStoreError::Database)?
            };
            for publication in &publications {
                store.connection.execute(
                    "UPDATE coding_inbox SET attempts=MIN(attempts+1,1000000),next_attempt_at=?3+MIN(300,1<<MIN(9,attempts+1)) WHERE accepted_source=?1 AND accepted_id=?2",
                    params![publication.identity.source, publication.identity.id, now],
                ).map_err(CleanStoreError::Database)?;
            }
            Ok(publications)
        })
    }
    pub(crate) fn record_coding_publication(
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
        let changed=self.connection.execute("UPDATE coding_inbox SET stored_at=?4,broker_sequence=?5 WHERE accepted_source=?1 AND accepted_id=?2 AND accepted_payload=?3 AND stored_at IS NULL",params![p.identity.source,p.identity.id,p.bytes,stored,sequence]).map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        Ok(())
    }
}
impl CleanStore {
    pub(crate) fn prune_coding_history(&mut self, now: i64) -> Result<(), CleanStoreError> {
        self.connection.execute("DELETE FROM coding_inbox WHERE rowid IN (SELECT i.rowid FROM coding_inbox i WHERE i.stored_at<?1 AND i.kind=1 AND NOT EXISTS(SELECT 1 FROM coding_executions e WHERE e.attempt_id=i.attempt_id AND (e.admission IS NULL OR json_extract(e.admission,'$.binding.generation')=i.generation)) ORDER BY i.stored_at LIMIT 128)",[now.saturating_sub(super::verification_outbox::HISTORY_SECONDS)]).map_err(CleanStoreError::Database)?;
        Ok(())
    }
    pub(super) fn validate_coding_events(&self) -> Result<(), CleanStoreError> {
        let mut q=self.connection.prepare("SELECT source,event_id,payload,decision,accepted_source,accepted_id,accepted_payload FROM coding_inbox").map_err(CleanStoreError::Database)?;
        let mut rows = q.query([]).map_err(CleanStoreError::Database)?;
        while let Some(r) = rows.next().map_err(CleanStoreError::Database)? {
            let identity = EventIdentity {
                source: r.get(0).map_err(CleanStoreError::Database)?,
                id: r.get(1).map_err(CleanStoreError::Database)?,
            };
            let bytes: Vec<u8> = r.get(2).map_err(CleanStoreError::Database)?;
            let payload = CodingReportAvailable::decode(bytes.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            coding::encode(&identity, &payload).map_err(|_| CleanStoreError::CorruptState)?;
            let decision: String = r.get(3).map_err(CleanStoreError::Database)?;
            if decision == "accepted" {
                let accepted = EventIdentity {
                    source: r.get(4).map_err(CleanStoreError::Database)?,
                    id: r.get(5).map_err(CleanStoreError::Database)?,
                };
                let bytes: Vec<u8> = r.get(6).map_err(CleanStoreError::Database)?;
                let (actual, fact) =
                    coding_decisions::decode(&bytes).map_err(|_| CleanStoreError::CorruptState)?;
                if actual != accepted
                    || fact.observation != Some(payload)
                    || fact.observation_source != identity.source
                    || fact.observation_id != identity.id
                {
                    return Err(CleanStoreError::CorruptState);
                }
            } else if !["pending", "rejected"].contains(&decision.as_str()) {
                return Err(CleanStoreError::CorruptState);
            }
        }
        Ok(())
    }
}
