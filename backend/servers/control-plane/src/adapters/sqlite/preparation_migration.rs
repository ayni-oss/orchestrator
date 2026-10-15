//! Upgrade durable preparation without discarding active work or retained evidence.
use super::{CleanStoreError, Connection, params};
use crate::profiles::ayni::preparation_state::RuntimeCursor;

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(CleanStoreError::Database)?;
    let rows = {
        let mut statement = transaction.prepare("SELECT name,value FROM reconciliation_cursors WHERE name LIKE 'managed:%' LIMIT 1025").map_err(CleanStoreError::Database)?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?
    };
    if rows.len() > 1024 {
        return Err(CleanStoreError::BoundExceeded);
    }
    for (name, value) in rows {
        if value.len() > 3 * 1_048_576 {
            return Err(CleanStoreError::BoundExceeded);
        }
        let cursor: RuntimeCursor =
            serde_json::from_str(&value).map_err(|_| CleanStoreError::CorruptState)?;
        let value = serde_json::to_string(&cursor).map_err(|_| CleanStoreError::CorruptState)?;
        transaction.execute("UPDATE reconciliation_cursors SET value=?1,resource_version=resource_version+1 WHERE name=?2", params![value, name]).map_err(CleanStoreError::Database)?;
    }
    super::summary_index::migrate(&transaction)?;
    // Earlier binaries reject this marker rather than misreading enum state as absent flags.
    transaction
        .execute(
            "UPDATE clean_schema SET version=10 WHERE component='control_plane'",
            [],
        )
        .map_err(CleanStoreError::Database)?;
    transaction.commit().map_err(CleanStoreError::Database)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::CleanStore;
    use crate::profiles::ayni::preparation_state::tests::legacy_cursor;
    use ayni_orchestrator_application::runtime_preparation::RetainedCredentials;

    #[test]
    fn upgrade_and_reopen_preserve_frozen_work_and_credentials()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("preparation.sqlite");
        let store = CleanStore::open(&path, 3)?;
        store.connection.execute_batch("DROP TABLE project_context_summaries; DROP TABLE project_context_records; DROP TABLE workspaces; DROP TABLE project_conversation_actions; DROP TABLE project_conversations; DROP TABLE project_agent_requests; DROP TABLE project_outcome_summaries; DROP TABLE lifecycle_outbox; DROP TABLE lifecycle_journal; DROP TABLE coding_inbox; DROP TABLE coding_executions; DROP TABLE runtime_operation_inbox; DROP TABLE verification_consumer; DROP TABLE verification_outbox; DROP TABLE verification_journal; DROP TABLE verification_inbox; DROP INDEX development_tasks_project_created_idx; ALTER TABLE development_tasks DROP COLUMN created_at_seconds; ALTER TABLE development_tasks DROP COLUMN created_at_nanos; UPDATE clean_schema SET version=9;")?;
        let original = legacy_cursor();
        let expected: RuntimeCursor = serde_json::from_value(original.clone())?;
        store.connection.execute(
            "INSERT INTO reconciliation_cursors VALUES('managed:profile',?1,7,21)",
            [original.to_string()],
        )?;
        drop(store);
        let store = CleanStore::open(&path, 3)?;
        assert_eq!(
            store.schema_version()?,
            crate::adapters::sqlite::SCHEMA_VERSION
        );
        let cursor = store.runtime_cursor("profile")?;
        assert_eq!(
            serde_json::to_value(&cursor)?,
            serde_json::to_value(&expected)?
        );
        assert_eq!(
            cursor.credential_references(),
            expected.credential_references()
        );
        let (version, updated): (i64, i64) = store.connection.query_row("SELECT resource_version,updated_at FROM reconciliation_cursors WHERE name='managed:profile'", [], |r| Ok((r.get(0)?,r.get(1)?)))?;
        assert_eq!((version, updated), (8, 21));
        drop(store);
        let store = CleanStore::open(&path, 3)?;
        assert_eq!(
            serde_json::to_value(store.runtime_cursor("profile")?)?,
            serde_json::to_value(&expected)?
        );
        Ok(())
    }

    #[test]
    fn malformed_preparation_rolls_back_every_cursor_and_schema_marker()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = CleanStore::in_memory()?;
        store
            .connection
            .execute("UPDATE clean_schema SET version=9", [])?;
        let original = legacy_cursor().to_string();
        store.connection.execute(
            "INSERT INTO reconciliation_cursors VALUES('managed:a-valid',?1,7,21)",
            [&original],
        )?;
        store.connection.execute(
            "INSERT INTO reconciliation_cursors VALUES('managed:z-corrupt','{}',1,21)",
            [],
        )?;
        assert!(matches!(
            migrate(&store.connection),
            Err(CleanStoreError::CorruptState)
        ));
        assert_eq!(store.schema_version()?, 9);
        let retained: String = store.connection.query_row(
            "SELECT value FROM reconciliation_cursors WHERE name='managed:a-valid'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(retained, original);
        Ok(())
    }
}
