use super::*;

fn workspace_request(repositories: Vec<String>) -> v1::CreateWorkspaceRequest {
    v1::CreateWorkspaceRequest {
        planner_idle_timeout_seconds: None,
        idempotency_key: "workspace".into(),
        display_name: "Payments workspace".into(),
        repository_connection_ids: repositories,
        agent_defaults: None,
    }
}
fn workspace_project(
    workspace: &v1::Workspace,
    repository: &str,
    key: &str,
) -> v1::CreateProjectRequest {
    v1::CreateProjectRequest {
        idempotency_key: key.into(),
        display_name: key.into(),
        workspace_id: workspace.id.clone(),
        goal: "Deliver the payment flow".into(),
        repository_connection_ids: vec![repository.into()],
        ..Default::default()
    }
}

#[test]
fn workspace_membership_gates_new_work_and_preserves_shared_runtime_and_admitted_work()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, artifact) = ready_profile(&mut store)?;
    let other = store.create_repository_connection("other", &repository_request("other", "456"))?;
    let workspace = store.create_workspace(
        "workspace",
        &workspace_request(vec![repository.id.clone(), other.id.clone()]),
    )?;
    let project =
        store.create_project("one", &workspace_project(&workspace, &repository.id, "one"))?;
    let sibling =
        store.create_project("two", &workspace_project(&workspace, &repository.id, "two"))?;
    let mut request = task_request(&store, &repository.id, &profile.id, "task")?;
    request
        .request
        .as_mut()
        .ok_or(CleanStoreError::CorruptState)?
        .project_id = project.id.clone();
    let (task, attempt) = store.create_development_task("task", &request)?;
    let conversation = store.create_project_conversation(
        "research",
        &v1::CreateProjectConversationRequest {
            planner_idle_timeout_seconds: None,
            idempotency_key: "research".into(),
            project_id: project.id.clone(),
            title: "Research".into(),
            repository_connection_ids: vec![repository.id.clone()],
        },
    )?;
    let update = v1::UpdateWorkspaceRequest {
        planner_idle_timeout_seconds: None,
        idempotency_key: "remove".into(),
        workspace_id: workspace.id.clone(),
        expected_resource_version: 1,
        display_name: workspace.display_name.clone(),
        repository_connection_ids: vec![other.id.clone()],
        agent_defaults: None,
    };
    store.update_workspace("remove", &update)?;
    assert_eq!(
        store.update_workspace("remove", &update)?.resource_version,
        2
    );
    assert!(matches!(
        store.update_workspace("stale", &update),
        Err(CleanStoreError::StaleVersion)
    ));
    assert!(matches!(
        store.create_development_task("new", &request),
        Err(CleanStoreError::ParentMismatch)
    ));
    assert!(matches!(
        store.send_project_message(
            "send",
            &v1::SendProjectMessageRequest {
                idempotency_key: "send".into(),
                conversation_id: conversation.id,
                content: "Investigate".into()
            },
            v1::ProjectAgentRun::default()
        ),
        Err(CleanStoreError::ParentMismatch)
    ));
    assert_eq!(
        store.create_development_task("task", &request)?.0.id,
        task.id
    );
    store.transition_attempt(&attempt.id, 1, v1::DevelopmentAttemptState::Running, None)?;
    store.transition_attempt(&attempt.id, 2, v1::DevelopmentAttemptState::Failed, None)?;
    assert!(matches!(
        store.retry_development_task(
            "retry",
            &v1::RetryDevelopmentTaskRequest {
                idempotency_key: "retry".into(),
                development_task_id: task.id.clone()
            }
        ),
        Err(CleanStoreError::ParentMismatch)
    ));
    assert_eq!(store.development_task(&task.id)?.snapshot, task.snapshot);
    assert_eq!(
        store.project(&sibling.id)?.repository_connection_ids,
        vec![repository.id.clone()]
    );
    assert_eq!(store.runtime_profile(&profile.id)?, profile);
    assert_eq!(store.runtime_artifact(&artifact.id)?, artifact);
    let profiles: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM runtime_profiles", [], |row| {
            row.get(0)
        })
        .map_err(CleanStoreError::Database)?;
    assert_eq!(profiles, 1);
    assert!(matches!(
        store.create_project(
            "removed",
            &workspace_project(&workspace, &repository.id, "removed")
        ),
        Err(CleanStoreError::ParentMismatch)
    ));
    let archived = store.archive_workspace(
        "archive",
        &v1::ArchiveWorkspaceRequest {
            idempotency_key: "archive".into(),
            workspace_id: workspace.id.clone(),
            expected_resource_version: 2,
        },
    )?;
    assert_eq!(archived.state, v1::WorkspaceState::Archived as i32);
    assert!(matches!(
        store.create_project(
            "archived",
            &workspace_project(&workspace, &other.id, "archived")
        ),
        Err(CleanStoreError::ProjectArchived)
    ));
    assert_eq!(store.project(&project.id)?.goal, "Deliver the payment flow");
    store.validate_durable_state()?;
    Ok(())
}

#[test]
fn workspace_defaults_are_inherited_by_pair_and_retained_as_connection_dependencies()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, _, _) = ready_profile(&mut store)?;
    let mut provider = v1::Connection {
        id: "provider-shared".into(),
        kind: "openai".into(),
        enabled: true,
        status: "ready".into(),
        ..Default::default()
    };
    store
        .connection
        .execute(
            "INSERT INTO connections(id,kind,payload) VALUES(?1,'openai',?2)",
            params![provider.id, provider.encode_to_vec()],
        )
        .map_err(CleanStoreError::Database)?;
    let mut request = workspace_request(vec![repository.id.clone()]);
    request.agent_defaults = Some(v1::ProjectAgentDefaults {
        planner_profile: None,
        worker_profile: None,
        project_connection_id: provider.id.clone(),
        project_model: "research-model".into(),
        coding_connection_id: provider.id.clone(),
        coding_model: "coding-model".into(),
    });
    let workspace = store.create_workspace("workspace", &request)?;
    let mut project = store.create_project(
        "project",
        &workspace_project(&workspace, &repository.id, "project"),
    )?;
    assert_eq!(
        store.effective_project_defaults(&project)?,
        workspace.agent_defaults
    );
    assert_eq!(store.resolve_task_provider(&project, "")?, provider.id);
    assert!(store.provider_connection_in_use(&provider.id)?);
    project.agent_defaults = Some(v1::ProjectAgentDefaults {
        project_connection_id: "override".into(),
        project_model: "override-model".into(),
        ..Default::default()
    });
    let effective = store
        .effective_project_defaults(&project)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(effective.project_model, "override-model");
    assert_eq!(effective.coding_model, "coding-model");
    provider.id = "unused".into();
    assert!(!store.provider_connection_in_use(&provider.id)?);
    Ok(())
}

#[test]
fn workspace_migration_preserves_frozen_work_and_is_restart_safe() -> Result<(), CleanStoreError> {
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("workspaces.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (repository, profile, artifact) = ready_profile(&mut store)?;
    let request = task_request(&store, &repository.id, &profile.id, "task")?;
    let (task, attempt) = store.create_development_task("task", &request)?;
    let mut project = store.project(&task.project_id)?;
    project.workspace_id.clear();
    project.agent_defaults = Some(v1::ProjectAgentDefaults {
        project_connection_id: "legacy-provider".into(),
        project_model: "legacy-model".into(),
        ..Default::default()
    });
    store
        .connection
        .execute(
            "UPDATE projects SET payload=?2 WHERE id=?1",
            params![project.id, project.encode_to_vec()],
        )
        .map_err(CleanStoreError::Database)?;
    store
        .connection
        .execute_batch("DROP TABLE project_worker_updates; DROP TABLE project_task_origins; DROP TABLE project_planner_sessions; DROP TABLE project_context_summaries; DROP TABLE project_context_records; DROP TABLE workspaces; UPDATE clean_schema SET version=19;")
        .map_err(CleanStoreError::Database)?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    let migrated = store.project(&project.id)?;
    assert_eq!(migrated.workspace_id, format!("workspace-{}", project.id));
    assert_eq!(
        store.workspace(&migrated.workspace_id)?.agent_defaults,
        project.agent_defaults
    );
    assert!(migrated.agent_defaults.is_none());
    assert_eq!(migrated.resource_version, project.resource_version);
    assert_eq!(store.development_task(&task.id)?, task);
    assert_eq!(store.development_attempt(&attempt.id)?, attempt);
    assert_eq!(store.runtime_profile(&profile.id)?, profile);
    assert_eq!(store.runtime_artifact(&artifact.id)?, artifact);
    assert_eq!(store.create_development_task("task", &request)?.0, task);
    let workspaces = store.list_workspaces(None, None)?.items;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(store.project(&project.id)?, migrated);
    assert_eq!(store.list_workspaces(None, None)?.items, workspaces);
    Ok(())
}

#[test]
fn corrupt_workspace_index_is_rejected_on_restart() -> Result<(), CleanStoreError> {
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("corrupt.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    ready_profile(&mut store)?;
    store
        .connection
        .execute("UPDATE workspaces SET state=2", [])
        .map_err(CleanStoreError::Database)?;
    drop(store);
    assert!(matches!(
        CleanStore::open(&path, 3),
        Err(CleanStoreError::CorruptState)
    ));
    Ok(())
}

#[test]
fn invalid_legacy_project_rolls_back_workspace_migration() -> Result<(), CleanStoreError> {
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("migration-rollback.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    ready_profile(&mut store)?;
    store.connection.execute_batch("DROP TABLE project_worker_updates; DROP TABLE project_task_origins; DROP TABLE project_planner_sessions; DROP TABLE project_context_summaries; DROP TABLE project_context_records; DROP TABLE workspaces; UPDATE clean_schema SET version=19; UPDATE projects SET payload=x'ff';").map_err(CleanStoreError::Database)?;
    drop(store);
    assert!(matches!(
        CleanStore::open(&path, 3),
        Err(CleanStoreError::CorruptState)
    ));
    let connection = Connection::open(&path).map_err(CleanStoreError::Database)?;
    let version: i64 = connection
        .query_row("SELECT version FROM clean_schema", [], |row| row.get(0))
        .map_err(CleanStoreError::Database)?;
    assert_eq!(version, 19);
    assert!(!existing_tables(&connection)?.contains(&"workspaces".into()));
    Ok(())
}
