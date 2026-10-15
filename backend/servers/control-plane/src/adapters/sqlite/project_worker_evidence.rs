//! Revision-fenced conversation projections of accepted attempts, independent of planner availability.
use super::{
    CleanStore, CleanStoreError, Connection, Digest, Message, OptionalExtension, Sha256, decode,
    params, v1,
};

impl CleanStore {
    pub(crate) fn sync_project_worker_updates(&mut self) -> Result<(), CleanStoreError> {
        let payloads=self.connection.prepare("SELECT a.payload FROM development_attempts a JOIN project_task_origins o ON o.task_id=a.development_task_id LEFT JOIN project_worker_updates u ON u.attempt_id=a.id WHERE a.resource_version>COALESCE(u.resource_version,0) ORDER BY a.id LIMIT 128").map_err(CleanStoreError::Database)?.query_map([],|r|r.get::<_,Vec<u8>>(0)).map_err(CleanStoreError::Database)?.collect::<Result<Vec<_>,_>>().map_err(CleanStoreError::Database)?;
        for payload in payloads {
            self.record_project_worker_update(&decode(&payload)?)?;
        }
        Ok(())
    }

    pub(super) fn record_project_worker_update(
        &mut self,
        attempt: &v1::DevelopmentAttempt,
    ) -> Result<(), CleanStoreError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            // A captured observation may race a newer accepted result before reconciliation.
            let current = self.development_attempt(&attempt.id)?;
            if current != *attempt {
                return Ok(());
            }
            let origin = super::project_origins::indexed_origin(
                &self.connection,
                &attempt.development_task_id,
            )?
            .ok_or(CleanStoreError::CorruptState)?;
            if attempt.snapshot.as_ref().and_then(|s| s.origin.as_ref()) != Some(&origin) {
                return Err(CleanStoreError::CorruptState);
            }
            let previous:Option<(i64,Vec<u8>)>=self.connection.query_row("SELECT resource_version,digest FROM project_worker_updates WHERE attempt_id=?1",[&attempt.id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(CleanStoreError::Database)?;
            let version = i64::try_from(attempt.resource_version)
                .map_err(|_| CleanStoreError::BoundExceeded)?;
            if previous.as_ref().is_some_and(|(old, _)| *old >= version) {
                return Ok(());
            }
            super::workflow_trace::outcome(attempt, &origin);
            let update = projection(attempt, origin.clone());
            let mut semantic = update.clone();
            semantic.resource_version = 0;
            semantic.updated_at = None;
            let digest = Sha256::digest(semantic.encode_to_vec()).to_vec();
            let changed = previous.as_ref().is_none_or(|(_, old)| *old != digest);
            if changed {
                let mut c = self.project_conversation(&origin.conversation_id)?;
                if c.project_id != origin.project_id {
                    return Err(CleanStoreError::CorruptState);
                }
                let outcome =
                    super::project_context::safe_outcome(&attempt.development_task_id, attempt);
                super::project_context::archive_outcome(&self.connection, &c, attempt, &outcome)?;
                self.save_conversation(&mut c)?;
            }
            self.connection.execute("INSERT INTO project_worker_updates(attempt_id,conversation_id,resource_version,digest,updated_at_seconds,payload) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(attempt_id) DO UPDATE SET resource_version=excluded.resource_version,digest=excluded.digest,updated_at_seconds=excluded.updated_at_seconds,payload=excluded.payload",params![attempt.id,origin.conversation_id,version,digest,attempt.updated_at.as_ref().map_or(0,|t|t.seconds),update.encode_to_vec()]).map_err(CleanStoreError::Database)?;
            Ok(())
        })();
        super::project_conversations::finish_transaction(&self.connection, result)
    }

    pub(super) fn conversation_worker_projection(
        &self,
        c: &mut v1::ProjectConversation,
    ) -> Result<(), CleanStoreError> {
        c.worker_updates.clear();
        let payloads=self.connection.prepare("SELECT payload FROM project_worker_updates WHERE conversation_id=?1 ORDER BY updated_at_seconds DESC,attempt_id DESC LIMIT 32").map_err(CleanStoreError::Database)?.query_map([&c.id],|r|r.get::<_,Vec<u8>>(0)).map_err(CleanStoreError::Database)?.collect::<Result<Vec<_>,_>>().map_err(CleanStoreError::Database)?;
        let mut bytes = 0;
        for payload in payloads {
            bytes += payload.len();
            if bytes > 1024 * 1024 {
                break;
            }
            c.worker_updates.push(decode(&payload)?);
        }
        Ok(())
    }

    pub(super) fn validate_project_worker_evidence(&self) -> Result<(), CleanStoreError> {
        let tasks = self
            .connection
            .prepare("SELECT payload FROM development_tasks")
            .map_err(CleanStoreError::Database)?
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        for bytes in tasks {
            let task: v1::DevelopmentTask = decode(&bytes)?;
            validate_snapshot_profile(task.snapshot.as_ref())?;
            let indexed = super::project_origins::indexed_origin(&self.connection, &task.id)?;
            if task.snapshot.as_ref().and_then(|s| s.origin.as_ref()) != indexed.as_ref() {
                return Err(CleanStoreError::CorruptState);
            }
            if let Some(origin) = indexed {
                let c = self.project_conversation(&origin.conversation_id)?;
                let proposal = c
                    .proposals
                    .iter()
                    .find(|p| {
                        p.id == origin.proposal_id
                            && p.task_id == task.id
                            && p.action == "create_task"
                            && p.state == "approved"
                    })
                    .ok_or(CleanStoreError::CorruptState)?;
                if task.project_id != origin.project_id
                    || super::project_origins::origin_for(&c, proposal)? != origin
                {
                    return Err(CleanStoreError::CorruptState);
                }
                validate_attempt_origins(&self.connection, &task.id, &origin)?;
            }
        }
        let updates=self.connection.prepare("SELECT attempt_id,conversation_id,resource_version,payload,digest FROM project_worker_updates").map_err(CleanStoreError::Database)?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,Vec<u8>>(3)?,r.get::<_,Vec<u8>>(4)?))).map_err(CleanStoreError::Database)?.collect::<Result<Vec<_>,_>>().map_err(CleanStoreError::Database)?;
        for (id, conversation, version, bytes, digest) in updates {
            let mut update: v1::ProjectWorkerUpdate = decode(&bytes)?;
            let attempt = self.development_attempt(&id)?;
            let origin = super::project_origins::indexed_origin(
                &self.connection,
                &attempt.development_task_id,
            )?;
            if update.attempt_id != id
                || update.task_id != attempt.development_task_id
                || update.origin != origin
                || origin
                    .as_ref()
                    .is_none_or(|o| o.conversation_id != conversation)
                || update.resource_version > attempt.resource_version
                || i64::try_from(update.resource_version) != Ok(version)
            {
                return Err(CleanStoreError::CorruptState);
            }
            update.resource_version = 0;
            update.updated_at = None;
            if Sha256::digest(update.encode_to_vec()).as_slice() != digest {
                return Err(CleanStoreError::CorruptState);
            }
        }
        Ok(())
    }
}

fn validate_attempt_origins(
    connection: &Connection,
    task: &str,
    origin: &v1::TaskOrigin,
) -> Result<(), CleanStoreError> {
    let attempts = connection
        .prepare("SELECT payload FROM development_attempts WHERE development_task_id=?1")
        .map_err(CleanStoreError::Database)?
        .query_map([task], |r| r.get::<_, Vec<u8>>(0))
        .map_err(CleanStoreError::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(CleanStoreError::Database)?;
    for bytes in attempts {
        let a: v1::DevelopmentAttempt = decode(&bytes)?;
        if let Some(snapshot) = &a.snapshot {
            crate::agent_profiles::validate_snapshot(snapshot)
                .map_err(|_| CleanStoreError::CorruptState)?;
        }
        if a.snapshot.as_ref().and_then(|s| s.origin.as_ref()) != Some(origin) {
            return Err(CleanStoreError::CorruptState);
        }
    }
    Ok(())
}
fn projection(a: &v1::DevelopmentAttempt, origin: v1::TaskOrigin) -> v1::ProjectWorkerUpdate {
    let mut result = a.result.clone();
    let mut progress = a.progress.clone();
    if let Some(progress) = &mut progress
        && let Some(quality) = &mut progress.latest_full_check
    {
        normalize_quality(quality, None);
    }
    if let Some(result) = &mut result {
        result.model_usage = None;
        if let Some(quality) = &mut result.quality {
            normalize_quality(quality, result.commit.as_ref());
        }
    }
    v1::ProjectWorkerUpdate {
        origin: Some(origin),
        task_id: a.development_task_id.clone(),
        attempt_id: a.id.clone(),
        attempt_number: a.attempt_number,
        resource_version: a.resource_version,
        state: a.state,
        requested_input: a.requested_input.clone(),
        progress,
        result,
        pull_request: a.pull_request.clone(),
        publication_only: a.publication_only,
        failure: a
            .failure
            .as_ref()
            .map_or_else(String::new, |f| f.message.clone()),
        updated_at: a.updated_at,
    }
}

/// Read projections never promote a bare retained outcome flag to verified quality.
pub(super) fn normalize_quality(quality: &mut v1::QualityResult, commit: Option<&String>) {
    let valid = quality.evidence.as_ref().is_some_and(|e| {
        super::payloads::valid_digest(&e.signals_digest)
            && super::payloads::valid_digest(&e.source_fingerprint)
            && super::valid_git_revision(&e.commit)
            && commit.is_none_or(|commit| commit == &e.commit)
    });
    if quality.outcome == v1::QualityOutcome::Passed as i32 && !valid {
        quality.outcome = v1::QualityOutcome::Unavailable as i32;
        quality.evidence = None;
    }
}

fn validate_snapshot_profile(
    snapshot: Option<&v1::DevelopmentSnapshot>,
) -> Result<(), CleanStoreError> {
    if let Some(snapshot) = snapshot {
        crate::agent_profiles::validate_snapshot(snapshot)
            .map_err(|_| CleanStoreError::CorruptState)?;
    }
    Ok(())
}
