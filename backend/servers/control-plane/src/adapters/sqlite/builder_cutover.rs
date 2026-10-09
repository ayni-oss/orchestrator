//! One-way retirement of the former installation lifecycle; only cleanup can resume.
use super::{CleanStore, CleanStoreError, Connection, params, unix_seconds};

/// Capture exact owned operation names before retiring obsolete control state.
pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    let version: i64 = connection
        .query_row(
            "SELECT version FROM clean_schema WHERE component='control_plane'",
            [],
            |row| row.get(0),
        )
        .map_err(CleanStoreError::Database)?;
    if version >= 9 {
        return Ok(());
    }

    let mut statement = connection
        .prepare("SELECT name,value FROM reconciliation_cursors WHERE name LIKE 'ayni:test:%' LIMIT 1025")
        .map_err(CleanStoreError::Database)?;
    let cursors = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(CleanStoreError::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(CleanStoreError::Database)?;
    if cursors.len() > 1024 {
        return Err(CleanStoreError::BoundExceeded);
    }
    let mut names = std::collections::BTreeSet::new();
    let mut engines = std::collections::BTreeSet::new();
    for (key, value) in cursors {
        let id = key
            .strip_prefix("ayni:test:")
            .ok_or(CleanStoreError::CorruptState)?;
        if id.is_empty()
            || id.len() > 63
            || !id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(CleanStoreError::CorruptState);
        }
        if value.len() > 3 * 1_048_576 {
            return Err(CleanStoreError::BoundExceeded);
        }
        let value: serde_json::Value =
            serde_json::from_str(&value).map_err(|_| CleanStoreError::CorruptState)?;
        let attempt = value["attempt"].as_u64().unwrap_or(0);
        if attempt > 64 {
            return Err(CleanStoreError::BoundExceeded);
        }
        for attempt in 0..=attempt {
            let name = if attempt == 0 {
                id.to_owned()
            } else {
                ayni_orchestrator_kubernetes::execution::operation_name(id, attempt)
            };
            engines.insert(name.clone());
            names.extend([
                format!("{name}-publish"),
                format!("{name}-discover"),
                format!("{name}-verify"),
                name,
            ]);
        }
    }
    collect_runtime_cleanup(connection, &mut names, &mut engines)?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(CleanStoreError::Database)?;
    for name in names {
        transaction.execute("INSERT OR IGNORE INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES(?1,?2,1,?3)",
            params![format!("retired-builder-cleanup:{}:{name}", if engines.contains(&name) { "engine" } else { "client" }), name, unix_seconds()?]).map_err(CleanStoreError::Database)?;
    }
    transaction.execute("DELETE FROM reconciliation_cursors WHERE name='ayni:build-service' OR name LIKE 'ayni:test:%' OR name LIKE 'ayni:operation:%'", []).map_err(CleanStoreError::Database)?;
    transaction.execute("DELETE FROM durable_actions WHERE operation IN ('InstallAyni','TestBuildService','CancelBuildServiceTest','UpdateBuildService','CancelAyniInstallation')", []).map_err(CleanStoreError::Database)?;
    transaction.execute("UPDATE reconciliation_cursors SET value=json_set(value,'$.pending',null,'$.trust',null,'$.preparation',null,'$.force_rebuild',json('false'),'$.next_at',0,'$.failures',0,'$.status',json('{}')) WHERE name LIKE 'managed:%'", []).map_err(CleanStoreError::Database)?;
    transaction
        .execute(
            "UPDATE clean_schema SET version=9 WHERE component='control_plane'",
            [],
        )
        .map_err(CleanStoreError::Database)?;
    transaction.commit().map_err(CleanStoreError::Database)
}

fn collect_runtime_cleanup(
    connection: &Connection,
    names: &mut std::collections::BTreeSet<String>,
    engines: &mut std::collections::BTreeSet<String>,
) -> Result<(), CleanStoreError> {
    // Pre-cutover runtime intents cannot execute under an obsolete builder contract.
    let mut managed = connection
        .prepare("SELECT value FROM reconciliation_cursors WHERE name LIKE 'managed:%' LIMIT 1025")
        .map_err(CleanStoreError::Database)?;
    let rows = managed
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(CleanStoreError::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(CleanStoreError::Database)?;
    if rows.len() > 1024 {
        return Err(CleanStoreError::BoundExceeded);
    }
    for row in rows {
        if row.len() > 3 * 1_048_576 {
            return Err(CleanStoreError::BoundExceeded);
        }
        let cursor: serde_json::Value =
            serde_json::from_str(&row).map_err(|_| CleanStoreError::CorruptState)?;
        for name in [
            cursor["pending"]["name"].as_str(),
            cursor["pending"]["build_name"].as_str(),
            cursor["pending"]["publish_name"].as_str(),
            cursor["trust"]["job"].as_str(),
        ]
        .into_iter()
        .flatten()
        {
            if name.is_empty()
                || name.len() > 63
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                return Err(CleanStoreError::CorruptState);
            }
            names.insert(name.into());
        }
        if let Some(name) = cursor["pending"]["build_name"].as_str() {
            engines.insert(name.to_owned());
        }
        if let Some(name) = cursor["pending"]["name"].as_str() {
            let attempt = cursor["pending"]["publish_attempts"].as_u64().unwrap_or(0);
            names.insert(ayni_orchestrator_kubernetes::execution::operation_name(
                &format!("discover:{name}"),
                attempt,
            ));
        }
    }
    Ok(())
}

impl CleanStore {
    pub(crate) fn retired_builder_cleanup(&self) -> Result<Vec<String>, CleanStoreError> {
        let mut statement = self.connection.prepare("SELECT value FROM reconciliation_cursors WHERE name LIKE 'retired-builder-cleanup:%' ORDER BY name LIMIT 128").map_err(CleanStoreError::Database)?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)
    }
    pub(crate) fn complete_retired_builder_cleanup(
        &self,
        name: &str,
    ) -> Result<(), CleanStoreError> {
        self.connection
            .execute(
                "DELETE FROM reconciliation_cursors WHERE name LIKE 'retired-builder-cleanup:%' AND value=?1",
                [name],
            )
            .map_err(CleanStoreError::Database)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cutover_retains_scoped_cleanup_until_acknowledged() -> Result<(), Box<dyn std::error::Error>>
    {
        let store = CleanStore::in_memory()?;
        store
            .connection
            .execute("UPDATE clean_schema SET version=8", [])?;
        store.connection.execute("INSERT INTO reconciliation_cursors VALUES ('ayni:test:runtime-example','{\"attempt\":1}',1,1)", [])?;
        store.connection.execute(
            "INSERT INTO reconciliation_cursors VALUES ('ayni:build-service','retired',1,1)",
            [],
        )?;
        migrate(&store.connection)?;
        let names = store.retired_builder_cleanup()?;
        assert_eq!(names.len(), 8);
        assert!(names.contains(&"runtime-example".to_owned()));
        migrate(&store.connection)?;
        assert_eq!(store.retired_builder_cleanup()?, names);
        for name in names {
            store.complete_retired_builder_cleanup(&name)?;
        }
        assert!(store.retired_builder_cleanup()?.is_empty());
        assert_eq!(store.schema_version()?, 9);
        Ok(())
    }
    #[test]
    fn cutover_stops_legacy_intents_without_touching_retained_work()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = CleanStore::in_memory()?;
        store
            .connection
            .execute("UPDATE clean_schema SET version=8", [])?;
        store.connection.execute(
            "INSERT INTO reconciliation_cursors VALUES ('coding:retained','retained-evidence',1,1)",
            [],
        )?;
        let cursor = serde_json::json!({"pending":{"name":"verify-owned","build_name":"build-owned","publish_name":"publish-owned","publish_attempts":0},"trust":{"job":"trust-owned"}});
        store.connection.execute(
            "INSERT INTO reconciliation_cursors VALUES ('managed:retired',?1,1,1)",
            [cursor.to_string()],
        )?;
        migrate(&store.connection)?;
        let names = store.retired_builder_cleanup()?;
        assert_eq!(names.len(), 5);
        assert_eq!(names.last().map(String::as_str), Some("build-owned"));
        let retained: String = store.connection.query_row(
            "SELECT value FROM reconciliation_cursors WHERE name='coding:retained'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(retained, "retained-evidence");
        let retired: String = store.connection.query_row(
            "SELECT value FROM reconciliation_cursors WHERE name='managed:retired'",
            [],
            |row| row.get(0),
        )?;
        let retired: serde_json::Value = serde_json::from_str(&retired)?;
        assert!(retired["pending"].is_null());
        assert!(retired["trust"].is_null());
        Ok(())
    }
}
