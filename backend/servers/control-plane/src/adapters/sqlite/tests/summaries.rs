use super::*;

#[test]
fn project_summary_uses_complete_history_and_preserves_member_readiness()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let mut newest = None;
    for index in 0..101 {
        let key = format!("task-{index}");
        let request = task_request(&store, &repository.id, &profile.id, &key)?;
        let (mut task, _) = store.create_development_task(&key, &request)?;
        // Distinct creation times make latest selection independent of random IDs.
        task.created_at = Some(timestamp(index));
        store
            .connection
            .execute(
                "UPDATE development_tasks SET payload=?2,created_at_seconds=?3 WHERE id=?1",
                params![task.id, task.encode_to_vec(), index],
            )
            .map_err(CleanStoreError::Database)?;
        newest = Some(task.id);
    }
    let summary = store.project_summaries(None)?.items.remove(0);
    assert!(summary.tasks_complete);
    assert_eq!(summary.latest_task.map(|task| task.id), newest);
    assert!(
        summary
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.complete && runtime.ready)
    );
    assert_eq!(summary.repository_names, vec![repository.display_name]);
    let other = store.create_repository_connection("other", &repository_request("other", "456"))?;
    let project = summary.project.ok_or(CleanStoreError::CorruptState)?;
    store.update_workspace(
        "add-workspace-member",
        &v1::UpdateWorkspaceRequest {
            idempotency_key: "add-workspace-member".into(),
            workspace_id: project.workspace_id.clone(),
            expected_resource_version: 1,
            display_name: project.display_name.clone(),
            repository_connection_ids: vec![repository.id.clone(), other.id.clone()],
            agent_defaults: None,
        },
    )?;
    store.update_project(
        "add-member",
        &v1::UpdateProjectRequest {
            goal: None,
            agent_defaults: None,
            idempotency_key: "add-member".into(),
            project_id: project.id,
            expected_resource_version: project.resource_version,
            display_name: project.display_name,
            repository_connection_ids: vec![repository.id, other.id],
        },
    )?;
    let summary = store.project_summaries(None)?.items.remove(0);
    assert!(
        summary.runtime.is_some_and(|runtime| runtime.complete
            && !runtime.ready
            && runtime.ready_profiles == 1)
    );
    Ok(())
}

#[test]
fn summary_pages_remain_frozen_after_profile_changes() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    store.create_project(
        "second",
        &v1::CreateProjectRequest {
            workspace_id: String::new(),
            goal: String::new(),
            idempotency_key: "second".into(),
            display_name: "Second".into(),
            repository_connection_id: repository.id,
            repository_connection_ids: vec![],
        },
    )?;
    let first = store.project_summaries(Some(&v1::PageRequest {
        page_size: 1,
        page_token: String::new(),
    }))?;
    assert!(
        first.items[0]
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.ready)
    );
    let mut changed = profile;
    changed.state = v1::RuntimeProfileState::Degraded as i32;
    store
        .connection
        .execute(
            "UPDATE runtime_profiles SET state=?2,payload=?3 WHERE id=?1",
            params![changed.id, changed.state, changed.encode_to_vec()],
        )
        .map_err(CleanStoreError::Database)?;
    let second = store.project_summaries(Some(&v1::PageRequest {
        page_size: 1,
        page_token: first.next_page_token.clone(),
    }))?;
    assert!(
        second.items[0]
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.ready)
    );
    assert!(second.next_page_token.is_empty());
    assert!(matches!(
        store.repository_summaries(Some(&v1::PageRequest {
            page_size: 1,
            page_token: first.next_page_token
        })),
        Err(CleanStoreError::InvalidPageToken)
    ));
    assert!(
        store.repository_summaries(None)?.items[0]
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.complete && !runtime.ready)
    );
    Ok(())
}

#[test]
fn corrupt_summary_input_fails_instead_of_reporting_empty_or_ready() -> Result<(), CleanStoreError>
{
    let mut store = CleanStore::in_memory()?;
    let (_, profile, _) = ready_profile(&mut store)?;
    store
        .connection
        .execute(
            "UPDATE runtime_profiles SET payload=?2 WHERE id=?1",
            params![profile.id, vec![0xff_u8]],
        )
        .map_err(CleanStoreError::Database)?;
    assert!(matches!(
        store.project_summaries(None),
        Err(CleanStoreError::CorruptState)
    ));
    assert!(matches!(
        store.repository_summaries(None),
        Err(CleanStoreError::CorruptState)
    ));
    Ok(())
}
