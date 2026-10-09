//! Index immutable task creation time once so summaries never scan task history.
use super::{CleanStoreError, Connection, Message, params, v1};

/// Called inside the schema migration's transaction; callers own commit/rollback.
pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection.execute_batch("ALTER TABLE development_tasks ADD COLUMN created_at_seconds INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE development_tasks ADD COLUMN created_at_nanos INTEGER NOT NULL DEFAULT 0;").map_err(CleanStoreError::Database)?;
    let mut statement = connection
        .prepare("SELECT id,payload FROM development_tasks")
        .map_err(CleanStoreError::Database)?;
    let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
    while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
        let id: String = row.get(0).map_err(CleanStoreError::Database)?;
        let payload: Vec<u8> = row.get(1).map_err(CleanStoreError::Database)?;
        let task = v1::DevelopmentTask::decode(payload.as_slice())
            .map_err(|_| CleanStoreError::CorruptState)?;
        let created = task.created_at.ok_or(CleanStoreError::CorruptState)?;
        if task.id != id || created.seconds < 0 || !(0..1_000_000_000).contains(&created.nanos) {
            return Err(CleanStoreError::CorruptState);
        }
        connection.execute("UPDATE development_tasks SET created_at_seconds=?2,created_at_nanos=?3 WHERE id=?1", params![id,created.seconds,created.nanos]).map_err(CleanStoreError::Database)?;
    }
    connection.execute_batch("CREATE INDEX development_tasks_project_created_idx ON development_tasks(project_id,created_at_seconds DESC,created_at_nanos DESC,id DESC);").map_err(CleanStoreError::Database)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_indexes_retained_timestamps_and_rolls_back_on_invalid_tasks()
    -> Result<(), CleanStoreError> {
        let connection = Connection::open_in_memory().map_err(CleanStoreError::Database)?;
        connection.execute_batch("CREATE TABLE development_tasks(id TEXT PRIMARY KEY,project_id TEXT NOT NULL,payload BLOB NOT NULL);").map_err(CleanStoreError::Database)?;
        let task = v1::DevelopmentTask {
            id: "task:1".into(),
            project_id: "project:1".into(),
            created_at: Some(super::super::timestamp(123)),
            ..Default::default()
        };
        connection
            .execute(
                "INSERT INTO development_tasks VALUES(?1,?2,?3)",
                params![task.id, task.project_id, task.encode_to_vec()],
            )
            .map_err(CleanStoreError::Database)?;
        {
            let transaction = connection
                .unchecked_transaction()
                .map_err(CleanStoreError::Database)?;
            migrate(&transaction)?;
            let time: (i64, i32) = transaction
                .query_row(
                    "SELECT created_at_seconds,created_at_nanos FROM development_tasks",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(CleanStoreError::Database)?;
            assert_eq!(time, (123, 0));
            // Intentionally roll back to exercise transactional DDL and data backfill.
        }
        connection
            .execute("UPDATE development_tasks SET payload=?1", [vec![0xff_u8]])
            .map_err(CleanStoreError::Database)?;
        {
            let transaction = connection
                .unchecked_transaction()
                .map_err(CleanStoreError::Database)?;
            assert!(matches!(
                migrate(&transaction),
                Err(CleanStoreError::CorruptState)
            ));
        }
        assert!(
            connection
                .prepare("SELECT created_at_seconds FROM development_tasks")
                .is_err()
        );
        Ok(())
    }
}
