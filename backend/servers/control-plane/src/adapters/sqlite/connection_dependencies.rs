//! Credential retention follows durable frozen work, including retryable tasks.
use super::{CleanStore, CleanStoreError, Message, v1};
impl CleanStore {
    fn connection_payloads(&self, table: &str) -> Result<Vec<Vec<u8>>, CleanStoreError> {
        let query = match table {
            "repository_connections" => "SELECT payload FROM repository_connections",
            "runtime_profiles" => "SELECT payload FROM runtime_profiles",
            "development_attempts" => "SELECT payload FROM development_attempts",
            _ => return Err(CleanStoreError::CorruptState),
        };
        let mut statement = self
            .connection
            .prepare(query)
            .map_err(CleanStoreError::Database)?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)
    }
    pub(crate) fn registry_connection_in_use(&self, id: &str) -> Result<bool, CleanStoreError> {
        let profiles = self
            .connection_payloads("runtime_profiles")?
            .into_iter()
            .map(|payload| {
                v1::RuntimeProfile::decode(payload.as_slice())
                    .map_err(|_| CleanStoreError::CorruptState)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for profile in &profiles {
            if self.runtime_registry(profile)?.registry_connection_id == id {
                return Ok(true);
            }
        }
        for payload in self.connection_payloads("repository_connections")? {
            let repository = v1::RepositoryConnection::decode(payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            // The legacy repository default only retains credentials while discovery
            // has no profile. Explicit runtime selections replace that authority.
            if repository.registry_connection_id == id
                && !profiles
                    .iter()
                    .any(|profile| profile.repository_connection_id == repository.id)
            {
                return Ok(true);
            }
        }
        for payload in self.connection_payloads("development_attempts")? {
            let attempt = v1::DevelopmentAttempt::decode(payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            if attempt
                .snapshot
                .and_then(|snapshot| snapshot.runtime)
                .is_some_and(|runtime| runtime.registry_connection_id == id)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(crate) fn connection_secret_in_use(&self, name: &str) -> Result<bool, CleanStoreError> {
        for payload in self.connection_payloads("development_attempts")? {
            let attempt = v1::DevelopmentAttempt::decode(payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            if attempt
                .snapshot
                .and_then(|snapshot| snapshot.runtime)
                .is_some_and(|runtime| runtime.pull_secret_name == name)
            {
                return Ok(true);
            }
        }
        let mut statement = self
            .connection
            .prepare("SELECT value FROM reconciliation_cursors WHERE name LIKE 'managed:%'")
            .map_err(CleanStoreError::Database)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(CleanStoreError::Database)?;
        for row in rows {
            let cursor: crate::profiles::ayni::preparation_state::RuntimeCursor =
                serde_json::from_str(&row.map_err(CleanStoreError::Database)?)
                    .map_err(|_| CleanStoreError::CorruptState)?;
            if ayni_orchestrator_application::runtime_preparation::RetainedCredentials::credential_references(&cursor).secrets.contains(name) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_retryable_history_retains_its_pull_credential_revision() -> Result<(), CleanStoreError>
    {
        let store = CleanStore::in_memory()?;
        // Isolate the dependency scan from unrelated project/provider enrollment.
        store
            .connection
            .execute_batch("PRAGMA foreign_keys=OFF")
            .map_err(CleanStoreError::Database)?;
        let attempt = v1::DevelopmentAttempt {
            id: "attempt".into(),
            development_task_id: "task".into(),
            attempt_number: 1,
            state: v1::DevelopmentAttemptState::Failed as i32,
            snapshot: Some(v1::DevelopmentSnapshot {
                runtime: Some(v1::FrozenRuntime {
                    registry_connection_id: "connection".into(),
                    pull_secret_name: "retired-pull".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            resource_version: 1,
            ..Default::default()
        };
        store.connection.execute("INSERT INTO development_attempts(id,development_task_id,attempt_number,state,payload,resource_version,event_cursor) VALUES('attempt','task',1,5,?1,1,1)",[attempt.encode_to_vec()]).map_err(CleanStoreError::Database)?;
        assert!(store.registry_connection_in_use("connection")?);
        assert!(store.connection_secret_in_use("retired-pull")?);
        assert!(!store.connection_secret_in_use("unrelated")?);
        store
            .connection
            .execute("DELETE FROM development_attempts", [])
            .map_err(CleanStoreError::Database)?;
        assert!(!store.registry_connection_in_use("connection")?);
        assert!(!store.connection_secret_in_use("retired-pull")?);
        Ok(())
    }
    #[test]
    fn pending_build_retains_both_credential_revisions() -> Result<(), CleanStoreError> {
        let store = CleanStore::in_memory()?;
        let cursor = crate::profiles::ayni::preparation_state::tests::legacy_cursor().to_string();
        store
            .connection
            .execute(
                "INSERT INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES('managed:profile',?1,1,unixepoch())",
                [cursor],
            )
            .map_err(CleanStoreError::Database)?;
        assert!(store.connection_secret_in_use("retired-pull")?);
        assert!(store.connection_secret_in_use("retired-push")?);
        assert!(!store.connection_secret_in_use("unreferenced")?);
        Ok(())
    }
}
