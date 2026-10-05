use super::*;

fn project_request(repository: &str, key: &str) -> v1::CreateProjectRequest {
    v1::CreateProjectRequest {
        idempotency_key: key.into(),
        display_name: "Payments".into(),
        repository_connection_id: repository.into(),
        repository_connection_ids: vec![],
    }
}

#[test]
fn project_membership_archive_and_reonboarding_preserve_history() -> Result<(), CleanStoreError> {
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("projects.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let repository =
        store.create_repository_connection("repo", &repository_request("repo", "123"))?;
    let request = project_request(&repository.id, "create");
    let project = store.create_project("create", &request)?;
    assert_eq!(store.create_project("create", &request)?.id, project.id);
    let shared = store.create_project("other", &project_request(&repository.id, "other"))?;
    assert_ne!(shared.id, project.id);
    let mut conflicting = request.clone();
    conflicting.display_name = "Other".into();
    assert!(matches!(
        store.create_project("create", &conflicting),
        Err(CleanStoreError::IdempotencyConflict)
    ));
    let rename = v1::UpdateProjectRequest {
        idempotency_key: "rename".into(),
        project_id: project.id.clone(),
        expected_resource_version: 1,
        display_name: "Ledger".into(),
        repository_connection_ids: vec![],
    };
    let renamed = store.update_project("rename", &rename)?;
    assert_eq!(renamed.display_name, "Ledger");
    assert_eq!(store.update_project("rename", &rename)?.resource_version, 2);
    assert!(matches!(
        store.update_project("stale", &rename),
        Err(CleanStoreError::StaleVersion)
    ));
    let archive = v1::ArchiveProjectRequest {
        idempotency_key: "archive".into(),
        project_id: project.id.clone(),
        expected_resource_version: 2,
    };
    let archived = store.archive_project("archive", &archive)?;
    assert_eq!(archived.state, v1::ProjectState::Archived as i32);
    assert_eq!(
        store.archive_project("archive", &archive)?.resource_version,
        3
    );
    assert!(matches!(
        store.project_member(&project.id, ""),
        Err(CleanStoreError::ProjectArchived)
    ));
    let next = store.create_project("next", &project_request(&repository.id, "next"))?;
    assert_ne!(next.id, project.id);
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(store.project(&project.id)?, archived);
    assert_eq!(
        store
            .list_projects(Some(v1::ProjectState::Active as i32), None)?
            .items,
        {
            let mut values = vec![next, shared];
            values.sort_by(|a, b| a.id.cmp(&b.id));
            values
        }
    );
    assert_eq!(
        store
            .list_projects(Some(v1::ProjectState::Archived as i32), None)?
            .items,
        vec![archived]
    );
    Ok(())
}

#[test]
fn project_task_admission_checks_membership_and_archive() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let request = task_request(&store, &repository.id, &profile.id, "task")?;
    let mut bad = request.clone();
    bad.request
        .as_mut()
        .ok_or(CleanStoreError::CorruptState)?
        .repository_connection_id = "repository:other".into();
    assert!(matches!(
        store.create_development_task("bad", &bad),
        Err(CleanStoreError::ParentMismatch)
    ));
    let (task, attempt) = store.create_development_task("task", &request)?;
    let project = store.project(&task.project_id)?;
    assert_eq!(project.repository_connection_ids, vec![repository.id]);
    store.transition_attempt(&attempt.id, 1, v1::DevelopmentAttemptState::Running, None)?;
    store.transition_attempt(&attempt.id, 2, v1::DevelopmentAttemptState::Failed, None)?;
    store.archive_project(
        "archive",
        &v1::ArchiveProjectRequest {
            idempotency_key: "archive".into(),
            project_id: project.id,
            expected_resource_version: 1,
        },
    )?;
    assert!(matches!(
        store.create_development_task("new", &request),
        Err(CleanStoreError::ProjectArchived)
    ));
    assert!(matches!(
        store.retry_development_task(
            "retry",
            &v1::RetryDevelopmentTaskRequest {
                idempotency_key: "retry".into(),
                development_task_id: task.id.clone()
            }
        ),
        Err(CleanStoreError::ProjectArchived)
    ));
    assert_eq!(
        store.development_task(&task.id)?.project_id,
        task.project_id
    );
    assert_eq!(
        store
            .list_development_tasks(Some(&task.project_id), None, None, None, None)?
            .items
            .len(),
        1
    );
    assert!(
        store
            .list_development_tasks(Some("project:other"), None, None, None, None)?
            .items
            .is_empty()
    );
    // A completed mutation replay remains readable after its project is archived.
    assert_eq!(
        store.create_development_task("task", &request)?.0.id,
        task.id
    );
    Ok(())
}

#[test]
fn project_pages_are_stable_and_bound_to_the_filter() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    for index in 1..=3 {
        let key = index.to_string();
        let repository =
            store.create_repository_connection(&key, &repository_request(&key, &key))?;
        store.create_project(&key, &project_request(&repository.id, &key))?;
    }
    let first = store.list_projects(
        None,
        Some(&v1::PageRequest {
            page_size: 1,
            page_token: String::new(),
        }),
    )?;
    let project = &first.items[0];
    store.archive_project(
        "archive",
        &v1::ArchiveProjectRequest {
            idempotency_key: "archive".into(),
            project_id: project.id.clone(),
            expected_resource_version: 1,
        },
    )?;
    let page = v1::PageRequest {
        page_size: 1,
        page_token: first.next_page_token,
    };
    assert!(matches!(
        store.list_projects(Some(v1::ProjectState::Active as i32), Some(&page)),
        Err(CleanStoreError::InvalidPageToken)
    ));
    let second = store.list_projects(None, Some(&page))?;
    assert_ne!(second.items[0].id, project.id);
    assert_eq!(second.snapshot_cursor, first.snapshot_cursor);
    Ok(())
}

#[test]
fn old_schema_is_rejected_without_migration() -> Result<(), CleanStoreError> {
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    for version in [1, 2, 3] {
        let path = directory.path().join(format!("old-{version}.sqlite"));
        let connection = Connection::open(&path).map_err(CleanStoreError::Database)?;
        connection.execute_batch("CREATE TABLE clean_schema(component TEXT, version INTEGER); CREATE TABLE retained_data(value TEXT); INSERT INTO retained_data VALUES ('preserve');").map_err(CleanStoreError::Database)?;
        connection
            .execute(
                "INSERT INTO clean_schema VALUES ('control_plane',?1)",
                [version],
            )
            .map_err(CleanStoreError::Database)?;
        assert!(
            matches!(CleanStore::open(&path, 3), Err(CleanStoreError::UnsupportedSchemaVersion(v)) if v == version)
        );
        let retained: String = connection
            .query_row("SELECT value FROM retained_data", [], |row| row.get(0))
            .map_err(CleanStoreError::Database)?;
        assert_eq!(retained, "preserve");
        assert_eq!(
            existing_tables(&connection)?,
            vec!["clean_schema", "retained_data"]
        );
    }
    Ok(())
}

#[test]
fn shared_membership_edits_preserve_task_history_and_block_removed_retries()
-> Result<(), CleanStoreError> {
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("shared.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let other = store.create_repository_connection("other", &repository_request("other", "456"))?;
    let mut request = task_request(&store, &repository.id, &profile.id, "task")?;
    let task_project = request
        .request
        .as_ref()
        .ok_or(CleanStoreError::CorruptState)?
        .project_id
        .clone();
    let shared = store.create_project(
        "shared",
        &v1::CreateProjectRequest {
            idempotency_key: "shared".into(),
            display_name: "Shared".into(),
            repository_connection_id: String::new(),
            repository_connection_ids: vec![repository.id.clone(), other.id.clone()],
        },
    )?;
    assert_eq!(
        store.project_member(&shared.id, &repository.id)?,
        repository.id
    );
    assert!(matches!(
        store.project_member(&shared.id, ""),
        Err(CleanStoreError::ParentMismatch)
    ));
    request
        .request
        .as_mut()
        .ok_or(CleanStoreError::CorruptState)?
        .project_id = shared.id.clone();
    let (task, attempt) = store.create_development_task("task", &request)?;
    store.transition_attempt(&attempt.id, 1, v1::DevelopmentAttemptState::Running, None)?;
    store.transition_attempt(&attempt.id, 2, v1::DevelopmentAttemptState::Failed, None)?;
    let edit = v1::UpdateProjectRequest {
        idempotency_key: "remove".into(),
        project_id: shared.id.clone(),
        expected_resource_version: 1,
        display_name: "Shared".into(),
        repository_connection_ids: vec![other.id.clone()],
    };
    let updated = store.update_project("remove", &edit)?;
    assert_eq!(updated.repository_connection_ids, vec![other.id.clone()]);
    assert_eq!(store.update_project("remove", &edit)?.resource_version, 2);
    assert!(matches!(
        store.update_project("stale", &edit),
        Err(CleanStoreError::StaleVersion)
    ));
    assert!(matches!(
        store.create_development_task("new", &request),
        Err(CleanStoreError::ParentMismatch)
    ));
    assert!(matches!(
        store.retry_development_task(
            "retry",
            &v1::RetryDevelopmentTaskRequest {
                idempotency_key: "retry".into(),
                development_task_id: task.id.clone(),
            }
        ),
        Err(CleanStoreError::ParentMismatch)
    ));
    // A different project sharing the same repository is unaffected.
    assert_eq!(
        store.project_member(&task_project, &repository.id)?,
        repository.id
    );
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(store.development_task(&task.id)?.project_id, shared.id);
    assert_eq!(
        store
            .list_development_tasks(Some(&shared.id), None, None, None, None)?
            .items
            .len(),
        1
    );
    assert!(
        store
            .list_development_tasks(Some(&task_project), None, None, None, None)?
            .items
            .is_empty()
    );
    // Re-adding a member reactivates its retained membership without duplicating it.
    store.update_project(
        "add",
        &v1::UpdateProjectRequest {
            idempotency_key: "add".into(),
            project_id: shared.id.clone(),
            expected_resource_version: 2,
            display_name: "Shared".into(),
            repository_connection_ids: vec![other.id, repository.id.clone()],
        },
    )?;
    store.retry_development_task(
        "restored",
        &v1::RetryDevelopmentTaskRequest {
            idempotency_key: "restored".into(),
            development_task_id: task.id,
        },
    )?;
    drop(store);
    let store = CleanStore::open(&path, 3)?;
    assert_eq!(
        store.project_member(&shared.id, &repository.id)?,
        repository.id
    );
    Ok(())
}

#[test]
fn project_membership_validation_rejects_duplicates_and_missing_connections()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let repository =
        store.create_repository_connection("repo", &repository_request("repo", "123"))?;
    let mut request = v1::CreateProjectRequest {
        idempotency_key: "project".into(),
        display_name: "Project".into(),
        repository_connection_id: String::new(),
        repository_connection_ids: vec![repository.id.clone(), repository.id.clone()],
    };
    assert!(api_validation::create_project(&request).is_err());
    assert!(store.create_project("duplicate", &request).is_err());
    request.repository_connection_ids = vec![repository.id, "repository:missing".into()];
    assert!(store.create_project("missing", &request).is_err());
    assert!(store.list_projects(None, None)?.items.is_empty());
    Ok(())
}
