use super::*;
impl CleanStore {
    pub(crate) fn list_connections(&self) -> Result<Vec<v1::Connection>, CleanStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM connections ORDER BY id")
            .map_err(CleanStoreError::Database)?;
        statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?
            .map(|row| {
                v1::Connection::decode(row.map_err(CleanStoreError::Database)?.as_slice())
                    .map_err(|_| CleanStoreError::CorruptState)
            })
            .collect()
    }
    pub(crate) fn get_connection(&self, id: &str) -> Result<v1::Connection, CleanStoreError> {
        self.list_connections()?
            .into_iter()
            .find(|value| value.id == id)
            .ok_or_else(|| CleanStoreError::NotFound {
                resource_type: "connection",
                resource_id: id.into(),
            })
    }
    pub(crate) fn save_connection(
        &mut self,
        value: &v1::Connection,
    ) -> Result<(), CleanStoreError> {
        let tx = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let previous: Option<Vec<u8>> = tx
            .query_row(
                "SELECT payload FROM connections WHERE id=?1",
                [&value.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        if let Some(previous) = previous {
            if previous == value.encode_to_vec() {
                return Ok(());
            }
            let previous = v1::Connection::decode(previous.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            if value.resource_version < previous.resource_version
                || value.resource_version > previous.resource_version + 1
            {
                return Err(CleanStoreError::StaleVersion);
            }
        }
        tx.execute("INSERT INTO connections(id,kind,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![value.id,value.kind,value.encode_to_vec()]).map_err(CleanStoreError::Database)?;
        append_event(
            &tx,
            "connections",
            "connection",
            &value.id,
            value.resource_version,
            None,
            None,
            value,
        )?;
        tx.commit().map_err(CleanStoreError::Database)?;
        Ok(())
    }
    pub(crate) fn connection_intent(
        &mut self,
        key: &str,
        value: &v1::Connection,
        operation: &str,
    ) -> Result<bool, CleanStoreError> {
        let version = i64::try_from(value.resource_version)
            .map_err(|_| CleanStoreError::InvalidTransition)?;
        let tx = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let prior:Option<(String,String,i64)>=tx.query_row("SELECT connection_id,operation,revision FROM connection_operations WHERE operation_key=?1",[key],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(CleanStoreError::Database)?;
        if let Some((id, op, revision)) = prior {
            if id != value.id || op != operation || revision != version {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return Ok(false);
        }
        let previous: Option<Vec<u8>> = tx
            .query_row(
                "SELECT payload FROM connections WHERE id=?1",
                [&value.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        if let Some(previous) = previous {
            let previous = v1::Connection::decode(previous.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            if previous.resource_version + 1 != value.resource_version {
                return Err(CleanStoreError::StaleVersion);
            }
        }
        tx.execute(
            "INSERT INTO connection_operations VALUES(?1,?2,?3,?4)",
            params![key, value.id, operation, version],
        )
        .map_err(CleanStoreError::Database)?;
        tx.execute("INSERT INTO connections(id,kind,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![value.id,value.kind,value.encode_to_vec()]).map_err(CleanStoreError::Database)?;
        append_event(
            &tx,
            "connections",
            "connection",
            &value.id,
            value.resource_version,
            None,
            None,
            value,
        )?;
        tx.commit().map_err(CleanStoreError::Database)?;
        Ok(true)
    }
    pub(crate) fn connection_operation_replayed(
        &self,
        key: &str,
        id: &str,
        version: u64,
        operation: &str,
    ) -> Result<bool, CleanStoreError> {
        let prior:Option<(String,String,i64)>=self.connection.query_row("SELECT connection_id,operation,revision FROM connection_operations WHERE operation_key=?1",[key],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(CleanStoreError::Database)?;
        if let Some((record_id, record_op, record_version)) = prior {
            if record_id != id
                || record_op != operation
                || record_version
                    != i64::try_from(version).map_err(|_| CleanStoreError::InvalidTransition)?
            {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return Ok(true);
        }
        Ok(false)
    }
    pub(crate) fn check_connection_deletion(&self, id: &str) -> Result<(), CleanStoreError> {
        let value = self.get_connection(id)?;
        if value.kind == "github_app" {
            let count: i64 = self
                .connection
                .query_row("SELECT COUNT(*) FROM repository_connections", [], |row| {
                    row.get(0)
                })
                .map_err(CleanStoreError::Database)?;
            if count > 0 {
                return Err(CleanStoreError::InvalidTransition);
            }
        }
        if value.kind == "ghcr" && self.registry_connection_in_use(id)? {
            return Err(CleanStoreError::InvalidTransition);
        }
        Ok(())
    }
    pub(crate) fn delete_connection(&mut self, id: &str) -> Result<(), CleanStoreError> {
        self.check_connection_deletion(id)?;
        let mut value = self.get_connection(id)?;
        value.resource_version = value
            .resource_version
            .checked_add(1)
            .ok_or(CleanStoreError::BoundExceeded)?;
        value.status = "deleted".into();
        let tx = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        append_event(
            &tx,
            "connections",
            "connection_deleted",
            id,
            value.resource_version,
            None,
            None,
            &value,
        )?;
        tx.execute("DELETE FROM connections WHERE id=?1", [id])
            .map_err(CleanStoreError::Database)?;
        tx.commit().map_err(CleanStoreError::Database)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn value(id: &str) -> v1::Connection {
        v1::Connection {
            id: id.into(),
            kind: "github_app".into(),
            management_source: "ui".into(),
            resource_version: 1,
            ..Default::default()
        }
    }
    #[test]
    fn only_one_github_app_can_be_stored() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        store.save_connection(&value("one"))?;
        assert!(store.save_connection(&value("two")).is_err());
        Ok(())
    }
    #[test]
    fn intents_are_atomic_and_reject_stale_replacements() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let mut connection = value("one");
        assert!(store.connection_intent("first", &connection, "create")?);
        assert!(!store.connection_intent("first", &connection, "create")?);
        assert!(matches!(
            store.connection_intent("first", &connection, "replace"),
            Err(CleanStoreError::IdempotencyConflict)
        ));
        connection.resource_version = 2;
        assert!(store.connection_intent("second", &connection, "replace")?);
        assert!(matches!(
            store.connection_intent("third", &connection, "replace"),
            Err(CleanStoreError::StaleVersion)
        ));
        Ok(())
    }
    #[test]
    fn records_and_intents_contain_only_safe_metadata() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let mut connection = value("one");
        connection.secret_name = "credential-reference".into();
        store.connection_intent("first", &connection, "create")?;
        assert_eq!(store.get_connection("one")?, connection);
        let operation: String = store
            .connection
            .query_row("SELECT operation FROM connection_operations", [], |row| {
                row.get(0)
            })
            .map_err(CleanStoreError::Database)?;
        assert_eq!(operation, "create");
        Ok(())
    }
}
