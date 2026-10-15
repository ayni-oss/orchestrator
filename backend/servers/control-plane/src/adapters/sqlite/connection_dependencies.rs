//! Credential retention follows durable frozen work, including retryable tasks.
use super::{CleanStore, CleanStoreError, Message, v1};
impl CleanStore {
    pub(crate) fn resolve_task_provider(
        &self,
        project: &v1::Project,
        requested: &str,
    ) -> Result<String, CleanStoreError> {
        if !requested.is_empty() {
            return Ok(requested.into());
        }
        if let Some(defaults) = self
            .effective_project_defaults(project)?
            .as_ref()
            .filter(|defaults| !defaults.coding_connection_id.is_empty())
        {
            return Ok(defaults.coding_connection_id.clone());
        }
        match self.get_connection("openai-admin") {
            Ok(_) => Ok("openai-admin".into()),
            Err(CleanStoreError::NotFound { .. }) => Ok(String::new()),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn freeze_task_provider(
        &self,
        id: &str,
    ) -> Result<Option<v1::FrozenProviderCredentials>, CleanStoreError> {
        if id.is_empty() {
            return Ok(None);
        }
        let value = self.get_connection(id)?;
        if value.kind != "openai"
            || !value.enabled
            || value.status != "ready"
            || value.secret_name.is_empty()
            || value.secret_uid.is_empty()
            || value.observed_revision.is_empty()
        {
            return Err(CleanStoreError::InvalidTransition);
        }
        Ok(Some(v1::FrozenProviderCredentials {
            connection_id: value.id,
            credential_revision: value.observed_revision,
            secret_name: value.secret_name,
            secret_uid: value.secret_uid,
        }))
    }
    fn connection_payloads(&self, table: &str) -> Result<Vec<Vec<u8>>, CleanStoreError> {
        let query = match table {
            "repository_connections" => "SELECT payload FROM repository_connections",
            "runtime_profiles" => "SELECT payload FROM runtime_profiles",
            "development_attempts" => "SELECT payload FROM development_attempts",
            "project_conversations" => "SELECT payload FROM project_conversations",
            "projects" => "SELECT payload FROM projects",
            "workspaces" => "SELECT payload FROM workspaces",
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
    pub(crate) fn provider_connection_in_use(&self, id: &str) -> Result<bool, CleanStoreError> {
        for payload in self.connection_payloads("workspaces")? {
            let workspace = v1::Workspace::decode(payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            if workspace.agent_defaults.is_some_and(|defaults| {
                defaults.project_connection_id == id || defaults.coding_connection_id == id
            }) {
                return Ok(true);
            }
        }
        for payload in self.connection_payloads("projects")? {
            let project = v1::Project::decode(payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            if project.agent_defaults.is_some_and(|defaults| {
                defaults.project_connection_id == id || defaults.coding_connection_id == id
            }) {
                return Ok(true);
            }
        }
        for payload in self.connection_payloads("development_attempts")? {
            let attempt = v1::DevelopmentAttempt::decode(payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            if attempt
                .snapshot
                .and_then(|snapshot| snapshot.provider_credentials)
                .is_some_and(|binding| binding.connection_id == id)
            {
                return Ok(true);
            }
        }
        for payload in self.connection_payloads("project_conversations")? {
            let conversation = v1::ProjectConversation::decode(payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            if conversation.runs.iter().any(|run| {
                run.provider_credentials
                    .as_ref()
                    .is_some_and(|binding| binding.connection_id == id)
            }) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(crate) fn connection_secret_in_use(&self, name: &str) -> Result<bool, CleanStoreError> {
        for payload in self.connection_payloads("project_conversations")? {
            let conversation = v1::ProjectConversation::decode(payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            if conversation.runs.iter().any(|run| {
                run.provider_credentials
                    .as_ref()
                    .is_some_and(|binding| binding.secret_name == name)
            }) {
                return Ok(true);
            }
        }
        for payload in self.connection_payloads("development_attempts")? {
            let attempt = v1::DevelopmentAttempt::decode(payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            if attempt
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.provider_credentials.as_ref())
                .is_some_and(|binding| binding.secret_name == name)
            {
                return Ok(true);
            }
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
            .prepare("SELECT value FROM reconciliation_cursors WHERE name LIKE 'managed:%' OR name LIKE 'repository-removal:%'")
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
    fn omitted_connection_resolves_admin_and_cannot_bypass_its_disabled_state()
    -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let project = v1::Project::default();
        assert_eq!(store.resolve_task_provider(&project, "")?, "");
        let mut admin = v1::Connection {
            id: "openai-admin".into(),
            kind: "openai".into(),
            management_source: "admin".into(),
            secret_name: "frozen-admin".into(),
            secret_uid: "uid".into(),
            observed_revision: "one".into(),
            status: "ready".into(),
            enabled: true,
            resource_version: 1,
            ..Default::default()
        };
        store.save_connection(&admin)?;
        let selected = store.resolve_task_provider(&project, "")?;
        assert_eq!(selected, "openai-admin");
        assert_eq!(
            store
                .freeze_task_provider(&selected)?
                .map(|binding| binding.secret_name),
            Some("frozen-admin".into())
        );
        admin.enabled = false;
        admin.resource_version += 1;
        store.save_connection(&admin)?;
        assert_eq!(store.resolve_task_provider(&project, "")?, "openai-admin");
        assert!(store.freeze_task_provider(&selected).is_err());
        assert_eq!(
            store.resolve_task_provider(&project, "explicit")?,
            "explicit"
        );
        let project = v1::Project {
            agent_defaults: Some(v1::ProjectAgentDefaults {
                coding_connection_id: "project-default".into(),
                coding_model: "model".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            store.resolve_task_provider(&project, "")?,
            "project-default"
        );
        Ok(())
    }

    #[test]
    fn provider_rotation_retains_admitted_binding_and_disabling_blocks_new_work()
    -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        assert!(store.freeze_task_provider("")?.is_none());
        let mut provider = v1::Connection {
            id: "openai-test".into(),
            kind: "openai".into(),
            management_source: "ui".into(),
            secret_name: "key-revision-one".into(),
            secret_uid: "uid-one".into(),
            observed_revision: "revision-one".into(),
            status: "ready".into(),
            enabled: true,
            resource_version: 1,
            ..Default::default()
        };
        store.save_connection(&provider)?;
        let binding = store
            .freeze_task_provider(&provider.id)?
            .ok_or(CleanStoreError::CorruptState)?;
        store
            .connection
            .execute_batch("PRAGMA foreign_keys=OFF")
            .map_err(CleanStoreError::Database)?;
        let attempt = v1::DevelopmentAttempt {
            id: "attempt-provider".into(),
            development_task_id: "task-provider".into(),
            attempt_number: 1,
            state: v1::DevelopmentAttemptState::Failed as i32,
            resource_version: 1,
            snapshot: Some(v1::DevelopmentSnapshot {
                provider_credentials: Some(binding.clone()),
                ..Default::default()
            }),
            ..Default::default()
        };
        store.connection.execute("INSERT INTO development_attempts(id,development_task_id,attempt_number,state,payload,resource_version,event_cursor) VALUES('attempt-provider','task-provider',1,5,?1,1,1)", [attempt.encode_to_vec()]).map_err(CleanStoreError::Database)?;
        provider.secret_name = "key-revision-two".into();
        provider.secret_uid = "uid-two".into();
        provider.observed_revision = "revision-two".into();
        provider.resource_version += 1;
        store.save_connection(&provider)?;
        let current = store
            .freeze_task_provider(&provider.id)?
            .ok_or(CleanStoreError::CorruptState)?;
        assert_eq!(current.secret_name, "key-revision-two");
        assert_eq!(binding.secret_name, "key-revision-one");
        assert!(store.connection_secret_in_use("key-revision-one")?);
        assert!(store.provider_connection_in_use("openai-test")?);
        assert!(store.check_connection_deletion("openai-test").is_err());
        provider.enabled = false;
        provider.resource_version += 1;
        store.save_connection(&provider)?;
        assert!(store.freeze_task_provider(&provider.id).is_err());
        assert!(store.connection_secret_in_use("key-revision-one")?);
        assert_eq!(
            store
                .development_attempt("attempt-provider")?
                .snapshot
                .and_then(|snapshot| snapshot.provider_credentials),
            Some(binding)
        );
        Ok(())
    }

    #[test]
    fn research_runs_retain_provider_credentials() -> Result<(), CleanStoreError> {
        let store = CleanStore::in_memory()?;
        store
            .connection
            .execute_batch("PRAGMA foreign_keys=OFF")
            .map_err(CleanStoreError::Database)?;
        let conversation = v1::ProjectConversation {
            id: "conversation".into(),
            project_id: "project".into(),
            runs: vec![v1::ProjectAgentRun {
                id: "run".into(),
                state: "interrupted".into(),
                provider_credentials: Some(v1::FrozenProviderCredentials {
                    connection_id: "openai-test".into(),
                    secret_name: "research-key".into(),
                    credential_revision: "one".into(),
                    secret_uid: "uid".into(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        store.connection.execute("INSERT INTO project_conversations(id,project_id,cursor,payload) VALUES('conversation','project',1,?1)", [conversation.encode_to_vec()]).map_err(CleanStoreError::Database)?;
        assert!(store.provider_connection_in_use("openai-test")?);
        assert!(store.connection_secret_in_use("research-key")?);
        assert!(!store.connection_secret_in_use("other-key")?);
        Ok(())
    }

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
