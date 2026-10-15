//! Approval-owned task provenance and additive backfill of retained execution snapshots.
use super::{CleanStoreError, Connection, Message, OptionalExtension, decode, params, v1};

pub(super) fn insert_origin(
    connection: &Connection,
    task: &str,
    origin: &v1::TaskOrigin,
) -> Result<(), CleanStoreError> {
    connection
        .execute(
            "INSERT INTO project_task_origins(task_id,conversation_id,payload) VALUES(?1,?2,?3)",
            params![task, origin.conversation_id, origin.encode_to_vec()],
        )
        .map_err(CleanStoreError::Database)?;
    Ok(())
}

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(CleanStoreError::Database)?;
    let result = (|| {
        connection.execute_batch("CREATE TABLE project_task_origins(task_id TEXT PRIMARY KEY REFERENCES development_tasks(id),conversation_id TEXT NOT NULL REFERENCES project_conversations(id),payload BLOB NOT NULL); CREATE INDEX project_task_origins_conversation ON project_task_origins(conversation_id); CREATE TABLE project_worker_updates(attempt_id TEXT PRIMARY KEY REFERENCES development_attempts(id),conversation_id TEXT NOT NULL REFERENCES project_conversations(id),resource_version INTEGER NOT NULL,digest BLOB NOT NULL,updated_at_seconds INTEGER NOT NULL,payload BLOB NOT NULL); CREATE INDEX project_worker_updates_conversation ON project_worker_updates(conversation_id);").map_err(CleanStoreError::Database)?;
        let conversations = connection
            .prepare("SELECT payload FROM project_conversations")
            .map_err(CleanStoreError::Database)?
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        for bytes in conversations {
            let mut c: v1::ProjectConversation = decode(&bytes)?;
            super::project_summary_retirement::retire(connection, &mut c)?;
            for p in c
                .proposals
                .iter()
                .filter(|p| p.action == "create_task" && p.state == "approved")
            {
                let origin = origin_for(&c, p)?;
                backfill(connection, &p.task_id, &origin)?;
            }
        }
        connection
            .execute_batch("DELETE FROM page_snapshots; UPDATE clean_schema SET version=24 WHERE component='control_plane'")
            .map_err(CleanStoreError::Database)
    })();
    super::project_conversations::finish_transaction(connection, result)
}

pub(super) fn origin_for(
    c: &v1::ProjectConversation,
    p: &v1::ProjectTaskProposal,
) -> Result<v1::TaskOrigin, CleanStoreError> {
    let plan = c.plan.as_ref().ok_or(CleanStoreError::CorruptState)?;
    let version = plan
        .versions
        .iter()
        .find(|v| v.proposal_ids.contains(&p.id))
        .ok_or(CleanStoreError::CorruptState)?;
    Ok(v1::TaskOrigin {
        project_id: c.project_id.clone(),
        conversation_id: c.id.clone(),
        plan_id: plan.id.clone(),
        plan_version: version.version,
        proposal_id: p.id.clone(),
        research_run_id: version.run_id.clone(),
    })
}

fn backfill(
    connection: &Connection,
    id: &str,
    origin: &v1::TaskOrigin,
) -> Result<(), CleanStoreError> {
    let bytes: Vec<u8> = connection
        .query_row(
            "SELECT payload FROM development_tasks WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(CleanStoreError::Database)?;
    let mut task: v1::DevelopmentTask = decode(&bytes)?;
    if task.project_id != origin.project_id {
        return Err(CleanStoreError::CorruptState);
    }
    let changed = task
        .snapshot
        .as_ref()
        .is_none_or(|snapshot| snapshot.origin.is_none());
    bind_snapshot(task.snapshot.as_mut(), origin)?;
    insert_origin(connection, id, origin)?;
    if changed {
        let previous = task.resource_version;
        task.resource_version = previous
            .checked_add(1)
            .ok_or(CleanStoreError::BoundExceeded)?;
        super::persist_task_update(connection, &task, previous)?;
    }
    let attempts = connection
        .prepare("SELECT payload FROM development_attempts WHERE development_task_id=?1")
        .map_err(CleanStoreError::Database)?
        .query_map([id], |r| r.get::<_, Vec<u8>>(0))
        .map_err(CleanStoreError::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(CleanStoreError::Database)?;
    for bytes in attempts {
        let mut attempt: v1::DevelopmentAttempt = decode(&bytes)?;
        let changed = attempt
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.origin.is_none());
        bind_snapshot(attempt.snapshot.as_mut(), origin)?;
        if changed {
            let previous = attempt.resource_version;
            attempt.resource_version = previous
                .checked_add(1)
                .ok_or(CleanStoreError::BoundExceeded)?;
            super::persist_attempt_update(connection, &attempt, previous, &task)?;
        }
    }
    Ok(())
}
fn bind_snapshot(
    snapshot: Option<&mut v1::DevelopmentSnapshot>,
    origin: &v1::TaskOrigin,
) -> Result<(), CleanStoreError> {
    let snapshot = snapshot.ok_or(CleanStoreError::CorruptState)?;
    if snapshot
        .origin
        .as_ref()
        .is_some_and(|existing| existing != origin)
    {
        return Err(CleanStoreError::CorruptState);
    }
    snapshot.origin = Some(origin.clone());
    Ok(())
}

pub(super) fn indexed_origin(
    connection: &Connection,
    task: &str,
) -> Result<Option<v1::TaskOrigin>, CleanStoreError> {
    let bytes: Option<Vec<u8>> = connection
        .query_row(
            "SELECT payload FROM project_task_origins WHERE task_id=?1",
            [task],
            |r| r.get(0),
        )
        .optional()
        .map_err(CleanStoreError::Database)?;
    bytes.map(|bytes| decode(&bytes)).transpose()
}
