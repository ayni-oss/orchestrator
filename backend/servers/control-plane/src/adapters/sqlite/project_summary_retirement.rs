//! Retire obsolete automatic model summaries without cancelling user research or coding.
use super::{CleanStoreError, Connection, Message, OptionalExtension, params, v1};

pub(super) fn retire(
    connection: &Connection,
    c: &mut v1::ProjectConversation,
) -> Result<(), CleanStoreError> {
    let mut retired = std::collections::BTreeSet::new();
    for run in &mut c.runs {
        if run.purpose == "summary" && matches!(run.state.as_str(), "queued" | "running") {
            run.state = "interrupted".into();
            run.failure =
                "Accepted worker evidence is returned directly to this conversation.".into();
            retired.insert(run.id.clone());
        }
    }
    if retired.is_empty() {
        return Ok(());
    }
    let raw: Option<String> = connection
        .query_row(
            "SELECT payload FROM project_planner_sessions WHERE conversation_id=?1",
            [&c.id],
            |r| r.get(0),
        )
        .optional()
        .map_err(CleanStoreError::Database)?;
    if let Some(raw) = raw {
        let mut session: super::project_sessions::PlannerSession =
            serde_json::from_str(&raw).map_err(|_| CleanStoreError::CorruptState)?;
        if retired.contains(&session.active_run_id) {
            session.state = "stopping".into();
            session.active_run_id.clear();
            session.idle_expires_at = None;
        }
        if retired.contains(&session.pending_run_id) {
            session.pending_run_id.clear();
        }
        connection
            .execute(
                "UPDATE project_planner_sessions SET payload=?1 WHERE conversation_id=?2",
                params![
                    serde_json::to_string(&session).map_err(|_| CleanStoreError::CorruptState)?,
                    c.id
                ],
            )
            .map_err(CleanStoreError::Database)?;
    }
    c.update_cursor = c
        .update_cursor
        .checked_add(1)
        .ok_or(CleanStoreError::BoundExceeded)?;
    connection
        .execute(
            "UPDATE project_conversations SET payload=?1,cursor=?2 WHERE id=?3",
            params![
                c.encode_to_vec(),
                i64::try_from(c.update_cursor).map_err(|_| CleanStoreError::BoundExceeded)?,
                c.id
            ],
        )
        .map_err(CleanStoreError::Database)?;
    Ok(())
}
