//! Global build settings use the existing versioned durable reconciliation store.
use super::*;
use base64::engine::general_purpose::STANDARD;
const SETTINGS: &str = "ayni:build-service";
impl CleanStore {
    pub(crate) fn build_settings(&self) -> Result<v1::BuildServiceSettings, CleanStoreError> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM reconciliation_cursors WHERE name=?1",
                [SETTINGS],
                |r| r.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        value
            .map(|value| {
                let bytes = STANDARD
                    .decode(value)
                    .map_err(|_| CleanStoreError::CorruptState)?;
                v1::BuildServiceSettings::decode(bytes.as_slice())
                    .map_err(|_| CleanStoreError::CorruptState)
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }
    pub(crate) fn save_build_settings(
        &mut self,
        settings: &v1::BuildServiceSettings,
    ) -> Result<(), CleanStoreError> {
        self.connection.execute("INSERT INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(name) DO UPDATE SET value=excluded.value,resource_version=excluded.resource_version,updated_at=excluded.updated_at",params![SETTINGS,STANDARD.encode(settings.encode_to_vec()),i64::try_from(settings.resource_version).map_err(|_|CleanStoreError::BoundExceeded)?,unix_seconds()?]).map_err(CleanStoreError::Database)?;
        Ok(())
    }
    pub(crate) fn build_operation(&self, key: &str, digest: &str) -> Result<bool, CleanStoreError> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM reconciliation_cursors WHERE name=?1",
                [format!("ayni:operation:{key}")],
                |r| r.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        match value {
            Some(value) if value != digest => Err(CleanStoreError::IdempotencyConflict),
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }
    pub(crate) fn save_build_operation(
        &mut self,
        key: &str,
        digest: &str,
        settings: &v1::BuildServiceSettings,
    ) -> Result<(), CleanStoreError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction.execute("INSERT INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES(?1,?2,1,?3)",params![format!("ayni:operation:{key}"),digest,unix_seconds()?]).map_err(CleanStoreError::Database)?;
        transaction.execute("INSERT INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(name) DO UPDATE SET value=excluded.value,resource_version=excluded.resource_version,updated_at=excluded.updated_at",params![SETTINGS,STANDARD.encode(settings.encode_to_vec()),i64::try_from(settings.resource_version).map_err(|_|CleanStoreError::BoundExceeded)?,unix_seconds()?]).map_err(CleanStoreError::Database)?;
        transaction.commit().map_err(CleanStoreError::Database)
    }
    pub(crate) fn build_test_cursor(&self, id: &str) -> Result<serde_json::Value, CleanStoreError> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM reconciliation_cursors WHERE name=?1",
                [format!("ayni:test:{id}")],
                |r| r.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        value
            .map(|v| serde_json::from_str(&v).map_err(|_| CleanStoreError::CorruptState))
            .transpose()
            .map(|v| v.unwrap_or_else(|| serde_json::json!({})))
    }
    pub(crate) fn save_build_test_cursor(
        &mut self,
        id: &str,
        value: &serde_json::Value,
    ) -> Result<(), CleanStoreError> {
        self.connection.execute("INSERT INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES(?1,?2,1,?3) ON CONFLICT(name) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",params![format!("ayni:test:{id}"),value.to_string(),unix_seconds()?]).map_err(CleanStoreError::Database)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_settings_and_operations_survive_reopening() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("state.sqlite");
        let settings = v1::BuildServiceSettings {
            resource_version: 3,
            status: "testing".into(),
            test_id: "test-attempt".into(),
            ..Default::default()
        };
        {
            let mut store = CleanStore::open(&path, 3)?;
            store.save_build_test_cursor(
                "test-attempt",
                &serde_json::json!({"stage":"publish","image_id":"candidate"}),
            )?;
            store.save_build_operation("operation", "digest", &settings)?;
            assert!(store.build_operation("operation", "digest")?);
            assert!(matches!(
                store.build_operation("operation", "other"),
                Err(CleanStoreError::IdempotencyConflict)
            ));
        }
        let store = CleanStore::open(&path, 3)?;
        assert_eq!(store.build_settings()?, settings);
        assert_eq!(store.build_test_cursor("test-attempt")?["stage"], "publish");
        Ok(())
    }
    #[test]
    fn schema_eight_accepts_global_settings_without_altering_retained_resources()
    -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        assert_eq!(store.schema_version()?, 8);
        assert_eq!(store.build_settings()?.resource_version, 0);
        store.save_build_settings(&v1::BuildServiceSettings {
            resource_version: 1,
            ..Default::default()
        })?;
        store.integrity_check()?;
        assert_eq!(store.schema_version()?, 8);
        Ok(())
    }
}
