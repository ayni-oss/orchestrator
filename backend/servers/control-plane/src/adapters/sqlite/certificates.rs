use super::*;
impl CleanStore {
    pub(crate) fn list_certificates(&self) -> Result<Vec<v1::Certificate>, CleanStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM certificates ORDER BY id")
            .map_err(CleanStoreError::Database)?;
        statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?
            .map(|row| {
                v1::Certificate::decode(row.map_err(CleanStoreError::Database)?.as_slice())
                    .map_err(|_| CleanStoreError::CorruptState)
            })
            .collect()
    }
    pub(crate) fn get_certificate(&self, id: &str) -> Result<v1::Certificate, CleanStoreError> {
        self.list_certificates()?
            .into_iter()
            .find(|value| value.id == id)
            .ok_or_else(|| CleanStoreError::NotFound {
                resource_type: "certificate",
                resource_id: id.into(),
            })
    }
    pub(crate) fn save_certificate(
        &mut self,
        value: &v1::Certificate,
    ) -> Result<(), CleanStoreError> {
        if value.is_default && !value.enabled {
            return Err(CleanStoreError::InvalidTransition);
        }
        match self.get_certificate(&value.id) {
            Ok(current) if current == *value => return Ok(()),
            Ok(current)
                if value.resource_version < current.resource_version
                    || value.resource_version > current.resource_version + 1 =>
            {
                return Err(CleanStoreError::StaleVersion);
            }
            Err(CleanStoreError::NotFound { .. }) if value.resource_version != 1 => {
                return Err(CleanStoreError::StaleVersion);
            }
            Err(error) if !matches!(error, CleanStoreError::NotFound { .. }) => return Err(error),
            _ => {}
        }
        let tx = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        tx.execute("INSERT INTO certificates(id,is_default,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET is_default=excluded.is_default,payload=excluded.payload",params![value.id,value.is_default,value.encode_to_vec()]).map_err(CleanStoreError::Database)?;
        append_event(
            &tx,
            "certificates",
            "certificate",
            &value.id,
            value.resource_version,
            None,
            None,
            value,
        )?;
        tx.commit().map_err(CleanStoreError::Database)?;
        Ok(())
    }
    pub(crate) fn certificate_operation(
        &self,
        key: &str,
        id: &str,
        op: &str,
        version: u64,
        fingerprint: &[u8],
    ) -> Result<bool, CleanStoreError> {
        let prior: Option<(String,String,i64,Vec<u8>)> = self.connection.query_row("SELECT certificate_id,operation,revision,fingerprint FROM certificate_operations WHERE operation_key=?1",[key],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(CleanStoreError::Database)?;
        if let Some((prior_id, prior_op, prior_version, prior_fingerprint)) = prior {
            if prior_id != id
                || prior_op != op
                || u64::try_from(prior_version).ok() != Some(version)
                || prior_fingerprint != fingerprint
            {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return Ok(true);
        }
        Ok(false)
    }
    pub(crate) fn certificate_commit(
        &mut self,
        value: &v1::Certificate,
        key: &str,
        op: &str,
        version: u64,
        fingerprint: &[u8],
    ) -> Result<(), CleanStoreError> {
        if value.is_default && !value.enabled {
            return Err(CleanStoreError::InvalidTransition);
        }
        let current = match self.get_certificate(&value.id) {
            Ok(current) => Some(current),
            Err(CleanStoreError::NotFound { .. }) => None,
            Err(error) => return Err(error),
        };
        if current
            .as_ref()
            .map_or(version != 0 || value.resource_version != 1, |current| {
                current.resource_version != version
                    || (op != "delete" && value.resource_version != version + 1)
            })
        {
            return Err(CleanStoreError::StaleVersion);
        }
        let tx = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        if value.is_default {
            let mut statement = tx
                .prepare("SELECT payload FROM certificates WHERE is_default=1 AND id<>?1")
                .map_err(CleanStoreError::Database)?;
            let old = statement
                .query_map([&value.id], |row| row.get::<_, Vec<u8>>(0))
                .map_err(CleanStoreError::Database)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(CleanStoreError::Database)?;
            drop(statement);
            for bytes in old {
                let mut old = v1::Certificate::decode(bytes.as_slice())
                    .map_err(|_| CleanStoreError::CorruptState)?;
                old.is_default = false;
                old.resource_version += 1;
                append_event(
                    &tx,
                    "certificates",
                    "certificate",
                    &old.id,
                    old.resource_version,
                    None,
                    None,
                    &old,
                )?;
                tx.execute(
                    "UPDATE certificates SET is_default=0,payload=?2 WHERE id=?1",
                    params![old.id, old.encode_to_vec()],
                )
                .map_err(CleanStoreError::Database)?;
            }
        }
        tx.execute(
            "INSERT INTO certificate_operations VALUES(?1,?2,?3,?4,?5)",
            params![
                key,
                value.id,
                op,
                i64::try_from(version).map_err(|_| CleanStoreError::InvalidTransition)?,
                fingerprint
            ],
        )
        .map_err(CleanStoreError::Database)?;
        if op == "delete" {
            let mut tombstone = value.clone();
            tombstone.resource_version = tombstone
                .resource_version
                .checked_add(1)
                .ok_or(CleanStoreError::BoundExceeded)?;
            append_event(
                &tx,
                "certificates",
                "certificate_deleted",
                &tombstone.id,
                tombstone.resource_version,
                None,
                None,
                &tombstone,
            )?;
            tx.execute("DELETE FROM certificates WHERE id=?1", [&value.id])
                .map_err(CleanStoreError::Database)?;
        } else {
            append_event(
                &tx,
                "certificates",
                "certificate",
                &value.id,
                value.resource_version,
                None,
                None,
                value,
            )?;
            tx.execute("INSERT INTO certificates(id,is_default,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET is_default=excluded.is_default,payload=excluded.payload",params![value.id,value.is_default,value.encode_to_vec()]).map_err(CleanStoreError::Database)?;
        }
        tx.commit().map_err(CleanStoreError::Database)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn value(id: &str) -> v1::Certificate {
        v1::Certificate {
            id: id.into(),
            enabled: true,
            is_default: true,
            resource_version: 1,
            public_key: "f".repeat(64),
            ..Default::default()
        }
    }
    #[test]
    fn schema_six_upgrade_preserves_connections_and_reopens()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("state.sqlite");
        let store = CleanStore::open(&path, 3)?;
        let connection = v1::Connection {
            id: "existing".into(),
            kind: "ghcr".into(),
            management_source: "ui".into(),
            resource_version: 1,
            ..Default::default()
        };
        store.connection.execute(
            "INSERT INTO connections(id,kind,payload) VALUES(?1,?2,?3)",
            params![connection.id, connection.kind, connection.encode_to_vec()],
        )?;
        store.connection.execute_batch("DROP TABLE project_conversation_actions; DROP TABLE project_conversations; DROP TABLE project_agent_requests; DROP TABLE project_outcome_summaries; DROP TABLE certificate_operations; DROP TABLE lifecycle_outbox; DROP TABLE lifecycle_journal; DROP TABLE coding_inbox; DROP TABLE coding_executions; DROP TABLE runtime_operation_inbox; DROP TABLE verification_consumer; DROP TABLE verification_outbox; DROP TABLE verification_journal; DROP TABLE verification_inbox; DROP TABLE certificates; CREATE TABLE automation_installations(id TEXT PRIMARY KEY); DROP INDEX development_tasks_project_created_idx; ALTER TABLE development_tasks DROP COLUMN created_at_seconds; ALTER TABLE development_tasks DROP COLUMN created_at_nanos; UPDATE clean_schema SET version=6;")?;
        drop(store);
        let store = CleanStore::open(&path, 3)?;
        assert_eq!(store.schema_version()?, super::SCHEMA_VERSION);
        assert_eq!(store.get_connection("existing")?, connection);
        assert!(store.list_certificates()?.is_empty());
        drop(store);
        let store = CleanStore::open(&path, 3)?;
        assert_eq!(store.get_connection("existing")?, connection);
        Ok(())
    }
    #[test]
    fn default_switch_and_operation_replay_are_durable() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        store.certificate_commit(&value("one"), "create-one", "create", 0, b"public-one")?;
        store.certificate_commit(&value("two"), "create-two", "create", 0, b"public-two")?;
        assert!(!store.get_certificate("one")?.is_default);
        assert!(store.get_certificate("two")?.is_default);
        assert!(store.certificate_operation("create-one", "one", "create", 0, b"public-one")?);
        assert!(matches!(
            store.certificate_operation("create-one", "one", "create", 0, b"other"),
            Err(CleanStoreError::IdempotencyConflict)
        ));
        assert_eq!(store.get_certificate("one")?.resource_version, 2);
        Ok(())
    }
    #[test]
    fn disabled_default_is_rejected() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let mut certificate = value("one");
        certificate.enabled = false;
        assert!(store.save_certificate(&certificate).is_err());
        assert!(
            store
                .certificate_commit(&certificate, "disabled", "create", 0, b"public")
                .is_err()
        );
        Ok(())
    }
}
