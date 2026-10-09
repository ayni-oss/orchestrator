//! Remove unused onboarding state atomically, retaining failed-operation cleanup.
use super::{
    CleanStore, CleanStoreError, Message, PROFILE_SERVICE, REPOSITORY_SERVICE, action_row,
    append_event, decode, map_constraint, params, request_digest, unix_seconds, v1,
};
use crate::profiles::ayni::preparation_state::RuntimeCursor;

impl CleanStore {
    pub(crate) fn remove_repository(
        &mut self,
        request: &v1::RemoveRepositoryRequest,
    ) -> Result<(), CleanStoreError> {
        let digest = request_digest("RemoveRepository", request);
        if let Some(action) = action_row(
            &self.connection,
            "RemoveRepository",
            &request.idempotency_key,
        )? {
            return if action.request_hash == digest {
                Ok(())
            } else {
                Err(CleanStoreError::IdempotencyConflict)
            };
        }
        let id = &request.repository_connection_id;
        let repository = self.repository_connection(id)?;
        if repository.resource_version != request.expected_resource_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let referenced: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM project_repositories WHERE repository_connection_id=?1)",
            [id], |row| row.get(0),
        ).map_err(CleanStoreError::Database)?;
        if referenced {
            return Err(CleanStoreError::RepositoryRemovalBlocked(
                "Remove this repository from its projects before removing its onboarding configuration. Archived project references are also retained.",
            ));
        }
        let tasks: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM development_tasks WHERE repository_connection_id=?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        if tasks {
            return Err(CleanStoreError::RepositoryRemovalBlocked(
                "This repository has retained tasks. Its runtime configuration cannot be removed while task history depends on it.",
            ));
        }
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM runtime_profiles WHERE repository_connection_id=?1")
            .map_err(CleanStoreError::Database)?;
        let profiles = statement
            .query_map([id], |row| row.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?
            .map(|row| decode::<v1::RuntimeProfile>(&row.map_err(CleanStoreError::Database)?))
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let discovery = match repository
            .provider
            .as_ref()
            .and_then(|provider| provider.provider.as_ref())
        {
            Some(v1::repository_provider::Provider::Github(github)) => {
                format!("{}/{}", github.owner, github.name)
            }
            None => return Err(CleanStoreError::CorruptState),
        };
        let mut cleanup = Vec::new();
        for profile in &profiles {
            let cursor = self.runtime_cursor(&profile.id)?;
            if cursor
                .pending
                .as_ref()
                .is_some_and(|pending| !pending.failed)
                || (cursor.failures < 3
                    && (cursor.builder.started_at.is_some()
                        || cursor
                            .trust
                            .as_ref()
                            .is_some_and(|trust| trust.state == "pending")))
            {
                return Err(CleanStoreError::RepositoryRemovalBlocked(
                    "Runtime preparation is still active. Wait for it to finish or fail, then retry removal.",
                ));
            }
            cleanup.push((
                profile.id.clone(),
                serde_json::to_string(&cursor).map_err(|_| CleanStoreError::CorruptState)?,
            ));
        }
        let now = unix_seconds()?;
        let tx = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        // The lifecycle write guard excludes discovery, analysis and runtime passes.
        // Retire interrupted intents so restart recovery cannot recreate deleted state.
        tx.execute(
            "UPDATE durable_actions SET state='failed',failure='repository_removed',updated_at=?3 WHERE state='pending' AND (target_id=?1 OR (operation='ConnectGithubRepository' AND lower(target_id)=lower(?2)) OR target_id IN (SELECT id FROM runtime_profiles WHERE repository_connection_id=?1))",
            params![id, discovery, now],
        ).map_err(CleanStoreError::Database)?;
        for (profile, cursor) in cleanup {
            tx.execute("INSERT INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES(?1,?2,1,?3)",
                params![format!("repository-removal:{profile}"), cursor, now]).map_err(CleanStoreError::Database)?;
            tx.execute(
                "DELETE FROM reconciliation_cursors WHERE name=?1",
                [format!("managed:{profile}")],
            )
            .map_err(CleanStoreError::Database)?;
        }
        for profile in &profiles {
            append_event(
                &tx,
                PROFILE_SERVICE,
                "runtime_profile_removed",
                &profile.id,
                profile
                    .resource_version
                    .checked_add(1)
                    .ok_or(CleanStoreError::BoundExceeded)?,
                Some(id),
                Some(&profile.id),
                &v1::RuntimeProfileRemoved {
                    runtime_profile_id: profile.id.clone(),
                    repository_connection_id: id.clone(),
                },
            )?;
        }
        append_event(
            &tx,
            REPOSITORY_SERVICE,
            "repository_removed",
            id,
            repository
                .resource_version
                .checked_add(1)
                .ok_or(CleanStoreError::BoundExceeded)?,
            Some(id),
            None,
            &v1::RepositoryRemoved {
                repository_connection_id: id.clone(),
            },
        )?;
        // Retain action receipts so old keys cannot recreate deleted identities.
        for sql in [
            "DELETE FROM runtime_artifacts WHERE runtime_profile_id IN (SELECT id FROM runtime_profiles WHERE repository_connection_id=?1)",
            "DELETE FROM runtime_profile_revisions WHERE runtime_profile_id IN (SELECT id FROM runtime_profiles WHERE repository_connection_id=?1)",
            "DELETE FROM runtime_profiles WHERE repository_connection_id=?1",
            "DELETE FROM repository_snapshots WHERE repository_connection_id=?1",
        ] {
            tx.execute(sql, [id]).map_err(CleanStoreError::Database)?;
        }
        let changed = tx
            .execute(
                "DELETE FROM repository_connections WHERE id=?1 AND resource_version=?2",
                params![
                    id,
                    i64::try_from(request.expected_resource_version)
                        .map_err(|_| CleanStoreError::BoundExceeded)?
                ],
            )
            .map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        tx.execute("INSERT INTO durable_actions(operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at) VALUES('RemoveRepository',?1,?2,?3,'completed','repository_connection',?4,?5,?5)",
            params![request.idempotency_key, digest, request.encode_to_vec(), id, now]).map_err(map_constraint)?;
        tx.commit().map_err(CleanStoreError::Database)
    }

    pub(crate) fn removed_runtime_cleanup(
        &self,
    ) -> Result<Vec<(String, RuntimeCursor)>, CleanStoreError> {
        let mut statement = self.connection.prepare("SELECT name,value FROM reconciliation_cursors WHERE name LIKE 'repository-removal:%' ORDER BY name LIMIT 128").map_err(CleanStoreError::Database)?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(CleanStoreError::Database)?
            .map(|row| {
                let (name, value) = row.map_err(CleanStoreError::Database)?;
                Ok((
                    name,
                    serde_json::from_str(&value).map_err(|_| CleanStoreError::CorruptState)?,
                ))
            })
            .collect()
    }

    pub(crate) fn complete_removed_runtime_cleanup(
        &self,
        name: &str,
    ) -> Result<(), CleanStoreError> {
        self.connection.execute("DELETE FROM reconciliation_cursors WHERE name=?1 AND name LIKE 'repository-removal:%'", [name])
            .map_err(CleanStoreError::Database)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect(
        store: &mut CleanStore,
        key: &str,
    ) -> Result<v1::RepositoryConnection, CleanStoreError> {
        store.create_repository_connection(
            key,
            &v1::CreateRepositoryConnectionRequest {
                idempotency_key: key.into(),
                display_name: "acme/sandbox".into(),
                default_branch: "main".into(),
                provider: Some(v1::RepositoryProvider {
                    provider: Some(v1::repository_provider::Provider::Github(
                        v1::GithubRepository {
                            repository_id: "123".into(),
                            owner: "acme".into(),
                            name: "sandbox".into(),
                            installation_id: 1,
                            html_url: "https://github.com/acme/sandbox".into(),
                        },
                    )),
                }),
                ..Default::default()
            },
        )
    }
    fn removal(repository: &v1::RepositoryConnection) -> v1::RemoveRepositoryRequest {
        v1::RemoveRepositoryRequest {
            idempotency_key: "remove".into(),
            repository_connection_id: repository.id.clone(),
            expected_resource_version: repository.resource_version,
        }
    }
    fn profile(
        store: &mut CleanStore,
        repository: &v1::RepositoryConnection,
    ) -> Result<v1::RuntimeProfile, CleanStoreError> {
        let snapshot = store.observe_repository_snapshot(&repository.id, &"a".repeat(40))?;
        Ok(store
            .create_runtime_profile(
                "runtime",
                &v1::CreateRuntimeProfileRequest {
                    idempotency_key: "runtime".into(),
                    repository_connection_id: repository.id.clone(),
                    repository_snapshot_id: snapshot.id,
                    configuration: Some(v1::RuntimeProfileConfiguration {
                        configuration: Some(
                            v1::runtime_profile_configuration::Configuration::AyniManaged(
                                v1::AyniManagedProfileRevision {
                                    lock_fingerprint: "b".repeat(64),
                                    ayni_version: "0.15.0".into(),
                                    oci_repository: "ghcr.io/acme/sandbox".into(),
                                    runtime_source_digest: None,
                                },
                            ),
                        ),
                    }),
                },
            )?
            .0)
    }

    #[test]
    fn repository_removal_cascades_and_reconnects_with_new_identity() -> Result<(), CleanStoreError>
    {
        let path = std::env::temp_dir().join(format!(
            "repository-removal-{}.sqlite",
            uuid::Uuid::now_v7()
        ));
        let mut store = CleanStore::open(&path, 2)?;
        let repository = connect(&mut store, "connect")?;
        let runtime = profile(&mut store, &repository)?;
        store.remove_repository(&removal(&repository))?;
        assert!(matches!(
            store.repository_connection(&repository.id),
            Err(CleanStoreError::NotFound { .. })
        ));
        assert!(matches!(
            store.runtime_profile(&runtime.id),
            Err(CleanStoreError::NotFound { .. })
        ));
        store.foreign_key_check()?;
        store.validate_durable_state()?;
        store.remove_repository(&removal(&repository))?;
        let replacement = connect(&mut store, "reconnect")?;
        assert_ne!(repository.id, replacement.id);
        assert_eq!(replacement.onboarding_status, "pending");
        assert!(matches!(
            connect(&mut store, "connect"),
            Err(CleanStoreError::NotFound { .. })
        ));
        drop(store);
        let reopened = CleanStore::open(&path, 2)?;
        assert_eq!(
            reopened.repository_connection(&replacement.id)?.id,
            replacement.id
        );
        assert_eq!(reopened.removed_runtime_cleanup()?.len(), 1);
        let cleanup_name = reopened.removed_runtime_cleanup()?[0].0.clone();
        reopened.complete_removed_runtime_cleanup(&cleanup_name)?;
        assert!(reopened.removed_runtime_cleanup()?.is_empty());
        reopened.foreign_key_check()?;
        drop(reopened);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn repository_removal_rejects_stale_versions_and_key_reuse() -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let repository = connect(&mut store, "connect")?;
        let mut request = removal(&repository);
        request.expected_resource_version += 1;
        assert!(matches!(
            store.remove_repository(&request),
            Err(CleanStoreError::StaleVersion)
        ));
        store.remove_repository(&removal(&repository))?;
        assert!(matches!(
            store.remove_repository(&request),
            Err(CleanStoreError::IdempotencyConflict)
        ));
        Ok(())
    }

    #[test]
    fn repository_removal_preserves_shared_project_and_task_references()
    -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let repository = connect(&mut store, "connect")?;
        let runtime = profile(&mut store, &repository)?;
        let project = store.create_project(
            "project",
            &v1::CreateProjectRequest {
                idempotency_key: "project".into(),
                display_name: "Sandbox".into(),
                repository_connection_ids: vec![repository.id.clone()],
                ..Default::default()
            },
        )?;
        store.create_project(
            "other-project",
            &v1::CreateProjectRequest {
                idempotency_key: "other-project".into(),
                display_name: "Shared sandbox".into(),
                repository_connection_ids: vec![repository.id.clone()],
                ..Default::default()
            },
        )?;
        assert!(matches!(
            store.remove_repository(&removal(&repository)),
            Err(CleanStoreError::RepositoryRemovalBlocked(_))
        ));
        // Isolate retained-task protection from the separately checked project memberships.
        store
            .connection
            .execute("DELETE FROM project_repositories", [])
            .map_err(CleanStoreError::Database)?;
        let task = v1::DevelopmentTask {
            id: "task-retained".into(),
            project_id: project.id.clone(),
            snapshot: Some(v1::DevelopmentSnapshot {
                repository: Some(v1::FrozenRepository {
                    repository_connection_id: repository.id.clone(),
                    ..Default::default()
                }),
                runtime: Some(v1::FrozenRuntime {
                    runtime_profile_id: runtime.id.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        store.connection.execute("INSERT INTO development_tasks(id,project_id,repository_connection_id,runtime_profile_id,state,payload,resource_version,event_cursor) VALUES(?1,?2,?3,?4,1,?5,1,1)", params![task.id, project.id, repository.id, runtime.id, task.encode_to_vec()]).map_err(CleanStoreError::Database)?;
        assert!(matches!(
            store.remove_repository(&removal(&repository)),
            Err(CleanStoreError::RepositoryRemovalBlocked(_))
        ));
        assert!(store.repository_connection(&repository.id).is_ok());
        assert!(store.runtime_profile(&runtime.id).is_ok());
        store.foreign_key_check()?;
        Ok(())
    }

    #[test]
    fn repository_removal_retires_interrupted_analysis_and_blocks_active_preparation()
    -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let repository = connect(&mut store, "connect")?;
        let runtime = profile(&mut store, &repository)?;
        let analysis = v1::AnalyzeRepositoryRequest {
            repository_connection_id: repository.id.clone(),
            idempotency_key: "analyze".into(),
        };
        store.begin_external_action(
            "AnalyzeRepository",
            "analyze",
            &analysis,
            "repository_connection",
            &repository.id,
        )?;

        let mut cursor = RuntimeCursor::default();
        cursor.builder.started_at = Some(1);
        cursor.builder.job = Some("builder-operation".into());
        store.save_runtime_cursor(&runtime.id, &cursor)?;
        assert!(matches!(
            store.remove_repository(&removal(&repository)),
            Err(CleanStoreError::RepositoryRemovalBlocked(_))
        ));
        cursor.failures = 3;
        store.save_runtime_cursor(&runtime.id, &cursor)?;
        store.remove_repository(&removal(&repository))?;
        assert_eq!(
            store
                .durable_action("AnalyzeRepository", "analyze")?
                .map(|action| action.state),
            Some(super::super::DurableActionState::Failed)
        );
        let cleanup = store.removed_runtime_cleanup()?;
        assert_eq!(
            cleanup[0].1.builder.job.as_deref(),
            Some("builder-operation")
        );
        store.validate_durable_state()?;
        Ok(())
    }
}
