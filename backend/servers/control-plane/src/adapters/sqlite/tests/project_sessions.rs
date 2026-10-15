use super::*;
use ayni_orchestrator_runtime::project_agent::ProjectAgentResult;
fn setup_session() -> Result<(CleanStore, v1::ProjectConversation), CleanStoreError> {
    let mut store = CleanStore::open(":memory:", 3)?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let request = task_request(&store, &repository.id, &profile.id, "session-task")?
        .request
        .ok_or(CleanStoreError::CorruptState)?;
    let c = store.create_project_conversation(
        "session-create",
        &v1::CreateProjectConversationRequest {
            planner_idle_timeout_seconds: None,
            idempotency_key: "session-create".into(),
            project_id: request.project_id,
            title: "Reusable planner".into(),
            repository_connection_ids: vec![repository.id],
        },
    )?;
    store.save_connection(&v1::Connection {
        id: "connection".into(),
        kind: "openai".into(),
        resource_version: 1,
        enabled: true,
        status: "ready".into(),
        secret_name: "planner-model".into(),
        secret_uid: "model-uid".into(),
        observed_revision: "1".into(),
        ..Default::default()
    })?;
    let project = store.project(&c.project_id)?;
    let workspace = store.workspace(&project.workspace_id)?;
    store.update_workspace(
        "session-agent-defaults",
        &v1::UpdateWorkspaceRequest {
            idempotency_key: "session-agent-defaults".into(),
            workspace_id: workspace.id,
            expected_resource_version: workspace.resource_version,
            display_name: workspace.display_name,
            agent_defaults: Some(v1::ProjectAgentDefaults {
                project_connection_id: "connection".into(),
                project_model: "model".into(),
                coding_connection_id: "connection".into(),
                coding_model: "model".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    Ok((store, c))
}
fn enqueue(
    store: &mut CleanStore,
    c: &v1::ProjectConversation,
    id: &str,
    revision: &str,
) -> Result<v1::SendProjectMessageRequest, CleanStoreError> {
    let message = v1::SendProjectMessageRequest {
        idempotency_key: id.into(),
        conversation_id: c.id.clone(),
        content: format!("Research {id}"),
    };
    let binding = v1::FrozenProviderCredentials {
        connection_id: "connection".into(),
        secret_name: "planner-model".into(),
        secret_uid: "model-uid".into(),
        credential_revision: revision.into(),
    };
    let mut provider = store.get_connection(&binding.connection_id)?;
    if provider.observed_revision != revision {
        provider.observed_revision = revision.into();
        store.save_connection(&provider)?;
    }
    store.send_project_message(
        id,
        &message,
        v1::ProjectAgentRun {
            id: id.into(),
            state: "queued".into(),
            model: "model".into(),
            connection_id: binding.connection_id.clone(),
            credential_revision: revision.into(),
            provider_credentials: Some(binding),
            purpose: "research".into(),
            session_managed: true,
            ..Default::default()
        },
    )?;
    Ok(message)
}
fn result(id: &str) -> ProjectAgentResult {
    ProjectAgentResult {
        run_id: id.into(),
        message: format!("Result {id}"),
        proposals: vec![],
        model_calls: 1,
        token_usage: ayni_orchestrator_domain::ProviderTokenUsage::default(),
    }
}
fn finish(
    store: &mut CleanStore,
    c: &v1::ProjectConversation,
    id: &str,
) -> Result<super::super::project_sessions::PlannerSession, CleanStoreError> {
    let (s, _) = store
        .prepare_planner_session(&c.id, id, 600)?
        .ok_or(CleanStoreError::CorruptState)?;
    store.bind_planner_pod(&c.id, s.generation, "resident-pod")?;
    store.mark_project_run_running(id)?;
    store.complete_planner_turn(&c.id, id, s.generation, s.turn_sequence, result(id))?;
    Ok(s)
}
#[test]
fn serialized_turns_reuse_session_and_expire_only_after_actual_idle() -> Result<(), CleanStoreError>
{
    let (mut store, c) = setup_session()?;
    let first = enqueue(&mut store, &c, "first", "1")?;
    let s = finish(&mut store, &c, "first")?;
    let idle = store
        .planner_session(&c.id)?
        .ok_or(CleanStoreError::CorruptState)?;
    let snapshot = store.project_conversation(&c.id)?;
    assert_eq!(snapshot.effective_planner_idle_timeout_seconds, 300);
    store.send_project_message("first", &first, v1::ProjectAgentRun::default())?;
    assert_eq!(
        store.planner_session(&c.id)?.map(|s| s.last_activity_at),
        Some(idle.last_activity_at)
    );
    enqueue(&mut store, &c, "second", "1")?;
    assert!(!store.expire_planner_session(&c.id, s.generation, i64::MAX)?);
    let (next, run) = store
        .prepare_planner_session(&c.id, "second", 600)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(next.session_id, s.session_id);
    assert_eq!(next.generation, s.generation);
    assert_eq!(next.turn_sequence, 2);
    assert_eq!(next.pod_uid, "resident-pod");
    let deadline = run.session_deadline_at;
    assert_eq!(
        store
            .prepare_planner_session(&c.id, "second", 999)?
            .map(|(_, r)| r.session_deadline_at),
        Some(deadline)
    );
    store.complete_planner_turn(
        &c.id,
        "second",
        next.generation,
        next.turn_sequence,
        result("second"),
    )?;
    let idle = store
        .planner_session(&c.id)?
        .ok_or(CleanStoreError::CorruptState)?;
    let expires = idle.idle_expires_at.ok_or(CleanStoreError::CorruptState)?;
    assert!(!store.expire_planner_session(&c.id, next.generation, expires - 1)?);
    assert!(store.expire_planner_session(&c.id, next.generation, expires)?);
    enqueue(&mut store, &c, "third", "1")?;
    assert!(
        store
            .prepare_planner_session(&c.id, "third", 600)?
            .is_none()
    );
    store.stop_planner_session(&c.id, next.generation, false)?;
    let (fresh, _) = store
        .prepare_planner_session(&c.id, "third", 600)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(fresh.generation, next.generation + 1);
    assert_ne!(fresh.session_id, next.session_id);
    assert!(
        store
            .project_conversation(&c.id)?
            .messages
            .iter()
            .any(|m| m.content == "Result first")
    );
    Ok(())
}
#[test]
fn credential_rotation_waits_for_owned_cleanup_and_fences_old_results()
-> Result<(), CleanStoreError> {
    let (mut store, c) = setup_session()?;
    enqueue(&mut store, &c, "first", "1")?;
    let old = finish(&mut store, &c, "first")?;
    enqueue(&mut store, &c, "second", "2")?;
    assert!(
        store
            .prepare_planner_session(&c.id, "second", 600)?
            .is_none()
    );
    assert_eq!(
        store.planner_session(&c.id)?.map(|s| s.state),
        Some("stopping".into())
    );
    store.stop_planner_session(&c.id, old.generation, false)?;
    let (fresh, _) = store
        .prepare_planner_session(&c.id, "second", 600)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(fresh.generation, old.generation + 1);
    assert_eq!(fresh.model_revision, "2");
    let before = store.project_conversation(&c.id)?;
    assert!(matches!(
        store.complete_planner_turn(
            &c.id,
            "second",
            old.generation,
            old.turn_sequence,
            result("second")
        ),
        Err(CleanStoreError::StaleVersion)
    ));
    assert_eq!(store.project_conversation(&c.id)?, before);
    store.complete_planner_turn(
        &c.id,
        "second",
        fresh.generation,
        fresh.turn_sequence,
        result("second"),
    )?;
    let completed = store.project_conversation(&c.id)?;
    store.complete_planner_turn(
        &c.id,
        "second",
        fresh.generation,
        fresh.turn_sequence,
        result("second"),
    )?;
    assert_eq!(store.project_conversation(&c.id)?, completed);
    Ok(())
}
#[test]
fn cancellation_survives_membership_removal_and_rejects_late_results() -> Result<(), CleanStoreError>
{
    let (mut store, c) = setup_session()?;
    enqueue(&mut store, &c, "cancel-me", "1")?;
    let (s, _) = store
        .prepare_planner_session(&c.id, "cancel-me", 600)?
        .ok_or(CleanStoreError::CorruptState)?;
    store.bind_planner_pod(&c.id, s.generation, "resident-pod")?;
    // Ownership remains valid after the workspace is archived; cancellation must still work.
    let project = store.project(&c.project_id)?;
    let mut workspace = store.workspace(&project.workspace_id)?;
    workspace.state = v1::WorkspaceState::Archived as i32;
    store
        .connection
        .execute(
            "UPDATE workspaces SET state=?1,payload=?2 WHERE id=?3",
            params![workspace.state, workspace.encode_to_vec(), workspace.id],
        )
        .map_err(CleanStoreError::Database)?;
    let cancel = v1::CancelProjectAgentRunRequest {
        idempotency_key: "cancel".into(),
        project_id: c.project_id.clone(),
        conversation_id: c.id.clone(),
        run_id: "cancel-me".into(),
    };
    let stopped = store.cancel_project_agent_run("cancel", &cancel)?;
    assert_eq!(stopped.runs[0].state, "cancelled");
    assert_eq!(
        stopped.planner_session.as_ref().map(|s| s.state.as_str()),
        Some("stopping")
    );
    assert_eq!(store.cancel_project_agent_run("cancel", &cancel)?, stopped);
    assert!(matches!(
        store.complete_planner_turn(
            &c.id,
            "cancel-me",
            s.generation,
            s.turn_sequence,
            result("cancel-me")
        ),
        Err(CleanStoreError::StaleVersion)
    ));
    Ok(())
}
#[test]
fn workspace_policy_changes_recompute_inherited_idle_deadlines_without_resetting_activity()
-> Result<(), CleanStoreError> {
    let (mut store, c) = setup_session()?;
    enqueue(&mut store, &c, "first", "1")?;
    let s = finish(&mut store, &c, "first")?;
    let idle = store
        .planner_session(&c.id)?
        .ok_or(CleanStoreError::CorruptState)?;
    let started = idle.idle_started_at.ok_or(CleanStoreError::CorruptState)?;
    let p = store.project(&c.project_id)?;
    let mut w = store.workspace(&p.workspace_id)?;
    w.planner_idle_timeout_seconds = 86400;
    store
        .connection
        .execute(
            "UPDATE workspaces SET payload=?1 WHERE id=?2",
            params![w.encode_to_vec(), w.id],
        )
        .map_err(CleanStoreError::Database)?;
    assert!(!store.expire_planner_session(&c.id, s.generation, started + 301)?);
    assert_eq!(
        store
            .planner_session(&c.id)?
            .and_then(|s| s.idle_expires_at),
        Some(started + 86400)
    );
    let current = store.project_conversation(&c.id)?;
    let updated = store.update_project_conversation_settings(
        "override",
        &v1::UpdateProjectConversationSettingsRequest {
            idempotency_key: "override".into(),
            project_id: c.project_id.clone(),
            conversation_id: c.id.clone(),
            expected_update_cursor: current.update_cursor,
            planner_idle_timeout_seconds: Some(300),
            inherit_workspace_timeout: false,
        },
    )?;
    assert_eq!(updated.effective_planner_idle_timeout_seconds, 300);
    assert_eq!(
        store.planner_session(&c.id)?.map(|s| s.last_activity_at),
        Some(idle.last_activity_at)
    );
    assert!(store.expire_planner_session(&c.id, s.generation, started + 301)?);
    Ok(())
}
#[test]
fn restart_preserves_active_turn_identity_and_deadline_and_lost_pod_requires_fresh_generation()
-> Result<(), CleanStoreError> {
    let (mut store, c) = setup_session()?;
    enqueue(&mut store, &c, "first", "1")?;
    let (s, bound) = store
        .prepare_planner_session(&c.id, "first", 600)?
        .ok_or(CleanStoreError::CorruptState)?;
    store.bind_planner_pod(&c.id, s.generation, "resident-pod")?;
    store.mark_project_run_running("first")?;
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("sessions.sqlite");
    store
        .connection
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .map_err(CleanStoreError::Database)?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    let (adopted, run) = store
        .prepare_planner_session(&c.id, "first", 999)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(adopted.session_id, s.session_id);
    assert_eq!(adopted.pod_uid, "resident-pod");
    assert_eq!(run.session_deadline_at, bound.session_deadline_at);
    store.stop_planner_session(&c.id, s.generation, true)?;
    assert_eq!(
        store.project_conversation(&c.id)?.runs[0].state,
        "interrupted"
    );
    enqueue(&mut store, &c, "next", "1")?;
    let (fresh, _) = store
        .prepare_planner_session(&c.id, "next", 600)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(fresh.generation, s.generation + 1);
    Ok(())
}

#[test]
fn lost_idle_resident_preserves_new_queued_message_and_replaces_before_dispatch()
-> Result<(), CleanStoreError> {
    let (mut store, c) = setup_session()?;
    enqueue(&mut store, &c, "first", "1")?;
    let old = finish(&mut store, &c, "first")?;
    enqueue(&mut store, &c, "next", "1")?;
    store.retire_idle_planner_session(&c.id, old.generation)?;
    assert!(store.prepare_planner_session(&c.id, "next", 600)?.is_none());
    store.stop_planner_session(&c.id, old.generation, true)?;
    assert_eq!(
        store
            .project_conversation(&c.id)?
            .runs
            .iter()
            .find(|r| r.id == "next")
            .map(|r| r.state.as_str()),
        Some("queued")
    );
    let (fresh, _) = store
        .prepare_planner_session(&c.id, "next", 600)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(fresh.generation, old.generation + 1);
    assert!(fresh.pod_uid.is_empty());
    // Once a queued turn has a pod identity, a lost pod may already have accepted it.
    store.bind_planner_pod(&c.id, fresh.generation, "possibly-accepted-pod")?;
    store.stop_planner_session(&c.id, fresh.generation, true)?;
    assert_eq!(
        store
            .project_conversation(&c.id)?
            .runs
            .iter()
            .find(|r| r.id == "next")
            .map(|r| r.state.as_str()),
        Some("interrupted")
    );
    assert!(store.prepare_planner_session(&c.id, "next", 600).is_err());
    Ok(())
}
#[test]
fn session_settings_are_scoped_versioned_and_accept_only_supported_timeouts()
-> Result<(), CleanStoreError> {
    let (mut store, c) = setup_session()?;
    let request = v1::UpdateProjectConversationSettingsRequest {
        idempotency_key: "settings".into(),
        project_id: c.project_id.clone(),
        conversation_id: c.id.clone(),
        expected_update_cursor: c.update_cursor,
        planner_idle_timeout_seconds: Some(86400),
        inherit_workspace_timeout: false,
    };
    assert!(
        store
            .update_project_conversation_settings(
                "wrong-project",
                &v1::UpdateProjectConversationSettingsRequest {
                    project_id: "different-project".into(),
                    ..request.clone()
                }
            )
            .is_err()
    );
    assert!(
        store
            .update_project_conversation_settings(
                "invalid",
                &v1::UpdateProjectConversationSettingsRequest {
                    planner_idle_timeout_seconds: Some(500),
                    ..request.clone()
                }
            )
            .is_err()
    );
    let updated = store.update_project_conversation_settings("settings", &request)?;
    assert_eq!(updated.effective_planner_idle_timeout_seconds, 86400);
    assert_eq!(
        store.update_project_conversation_settings("settings", &request)?,
        updated
    );
    assert!(matches!(
        store.update_project_conversation_settings("stale", &request),
        Err(CleanStoreError::StaleVersion)
    ));
    let inherited = store.update_project_conversation_settings(
        "inherit",
        &v1::UpdateProjectConversationSettingsRequest {
            expected_update_cursor: updated.update_cursor,
            planner_idle_timeout_seconds: None,
            inherit_workspace_timeout: true,
            ..request
        },
    )?;
    assert_eq!(inherited.planner_idle_timeout_seconds, None);
    assert_eq!(inherited.effective_planner_idle_timeout_seconds, 300);
    Ok(())
}
#[test]
fn schema_twenty_two_upgrade_preserves_legacy_frozen_requests_and_receipts()
-> Result<(), CleanStoreError> {
    let (mut store, c) = setup_session()?;
    let request = v1::SendProjectMessageRequest {
        idempotency_key: "legacy".into(),
        conversation_id: c.id.clone(),
        content: "Keep frozen legacy input".into(),
    };
    let legacy = v1::ProjectAgentRun {
        id: "legacy-run".into(),
        state: "queued".into(),
        model: "legacy-model".into(),
        ..Default::default()
    };
    store.send_project_message("legacy", &request, legacy)?;
    let frozen = ayni_orchestrator_runtime::project_agent::ProjectAgentRequest {
        worker_profile: None,
        profile: None,
        run_id: "legacy-run".into(),
        project_id: c.project_id.clone(),
        model: "legacy-model".into(),
        history: "original".into(),
        repositories: vec![],
        context: "original context".into(),
        conversation_context: serde_json::Value::Null,
        max_model_calls: 1,
        deadline_seconds: 600,
    };
    store.save_project_run_request("legacy-run", &frozen)?;
    let original: String = store
        .connection
        .query_row(
            "SELECT payload FROM project_agent_requests WHERE run_id='legacy-run'",
            [],
            |r| r.get(0),
        )
        .map_err(CleanStoreError::Database)?;
    store
        .connection
        .execute_batch("DROP TABLE project_worker_updates; DROP TABLE project_task_origins; DROP TABLE project_planner_sessions; UPDATE clean_schema SET version=22;")
        .map_err(CleanStoreError::Database)?;
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("legacy.sqlite");
    store
        .connection
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .map_err(CleanStoreError::Database)?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(store.schema_version()?, SCHEMA_VERSION);
    let actual: String = store
        .connection
        .query_row(
            "SELECT payload FROM project_agent_requests WHERE run_id='legacy-run'",
            [],
            |r| r.get(0),
        )
        .map_err(CleanStoreError::Database)?;
    assert_eq!(actual, original);
    let replay = store.send_project_message("legacy", &request, v1::ProjectAgentRun::default())?;
    assert!(!replay.runs[0].session_managed);
    assert!(replay.runs[0].session_id.is_empty());
    assert!(replay.planner_session.is_none());
    assert_eq!(replay.effective_planner_idle_timeout_seconds, 300);
    store.complete_project_run(result("legacy-run"))?;
    assert_eq!(
        store.project_conversation(&c.id)?.runs[0].state,
        "completed"
    );
    Ok(())
}

#[test]
fn queued_planner_turn_is_found_after_more_than_one_thousand_cold_conversations()
-> Result<(), CleanStoreError> {
    let (mut store, c) = setup_session()?;
    // Cold retained conversations must not consume the active reconcile batch.
    for index in 0..1100 {
        let mut cold = c.clone();
        cold.id = format!("cold-{index:04}");
        store
            .connection
            .execute(
                "INSERT INTO project_conversations VALUES(?1,?2,?3,?4)",
                params![
                    cold.id,
                    cold.project_id,
                    i64::try_from(cold.update_cursor).map_err(|_| CleanStoreError::CorruptState)?,
                    cold.encode_to_vec()
                ],
            )
            .map_err(CleanStoreError::Database)?;
    }
    enqueue(&mut store, &c, "waiting-after-cold-history", "1")?;
    let queued = store.queued_project_runs()?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].1.id, "waiting-after-cold-history");
    assert_eq!(queued[0].0.id, c.id);
    Ok(())
}

#[test]
fn separate_connections_admit_only_one_concurrent_planner_turn() -> Result<(), CleanStoreError> {
    let (store, c) = setup_session()?;
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("serial.sqlite");
    store
        .connection
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .map_err(CleanStoreError::Database)?;
    drop(store);
    std::thread::scope(|scope| -> Result<(), CleanStoreError> {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = vec![];
        for id in ["concurrent-first", "concurrent-second"] {
            let path = path.clone();
            let c = c.clone();
            let barrier = barrier.clone();
            handles.push(scope.spawn(move || {
                let mut store = CleanStore::open(&path, 3)?;
                barrier.wait();
                enqueue(&mut store, &c, id, "1")
            }));
        }
        let mut admitted = 0;
        let mut rejected = 0;
        for handle in handles {
            match handle.join().map_err(|_| CleanStoreError::CorruptState)? {
                Ok(_) => admitted += 1,
                Err(CleanStoreError::InvalidTransition) => rejected += 1,
                Err(error) => return Err(error),
            }
        }
        assert_eq!(admitted, 1);
        assert_eq!(rejected, 1);
        Ok(())
    })?;
    let store = CleanStore::open(&path, 3)?;
    let actual = store.project_conversation(&c.id)?;
    assert_eq!(actual.runs.len(), 1);
    assert_eq!(actual.messages.len(), 1);
    assert_eq!(
        store
            .project_conversation_context(&c.project_id, &c.id)?
            .archive_record_count,
        1
    );
    Ok(())
}
#[test]
fn concurrent_settings_and_cancellation_never_resurrect_a_cancelled_turn()
-> Result<(), CleanStoreError> {
    let (mut store, c) = setup_session()?;
    enqueue(&mut store, &c, "cancel-race", "1")?;
    let (session, _) = store
        .prepare_planner_session(&c.id, "cancel-race", 600)?
        .ok_or(CleanStoreError::CorruptState)?;
    store.bind_planner_pod(&c.id, session.generation, "resident-pod")?;
    let c = store.project_conversation(&c.id)?;
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("cancel-race.sqlite");
    store
        .connection
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .map_err(CleanStoreError::Database)?;
    drop(store);
    std::thread::scope(|scope| -> Result<(), CleanStoreError> {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let settings_path = path.clone();
        let settings_c = c.clone();
        let settings_barrier = barrier.clone();
        let settings = scope.spawn(move || {
            let mut store = CleanStore::open(&settings_path, 3)?;
            settings_barrier.wait();
            store.update_project_conversation_settings(
                "racing-settings",
                &v1::UpdateProjectConversationSettingsRequest {
                    idempotency_key: "racing-settings".into(),
                    project_id: settings_c.project_id,
                    conversation_id: settings_c.id,
                    expected_update_cursor: settings_c.update_cursor,
                    planner_idle_timeout_seconds: Some(86400),
                    inherit_workspace_timeout: false,
                },
            )
        });
        let cancel_path = &path;
        let cancel_c = &c;
        let cancel = scope.spawn(move || {
            let mut store = CleanStore::open(cancel_path, 3)?;
            barrier.wait();
            store.cancel_project_agent_run(
                "racing-cancel",
                &v1::CancelProjectAgentRunRequest {
                    idempotency_key: "racing-cancel".into(),
                    project_id: cancel_c.project_id.clone(),
                    conversation_id: cancel_c.id.clone(),
                    run_id: "cancel-race".into(),
                },
            )
        });
        cancel.join().map_err(|_| CleanStoreError::CorruptState)??;
        match settings.join().map_err(|_| CleanStoreError::CorruptState)? {
            Ok(_) | Err(CleanStoreError::StaleVersion) => {}
            Err(error) => return Err(error),
        }
        Ok(())
    })?;
    let mut store = CleanStore::open(&path, 3)?;
    let actual = store.project_conversation(&c.id)?;
    assert_eq!(actual.runs[0].state, "cancelled");
    assert_eq!(
        actual.planner_session.as_ref().map(|s| s.state.as_str()),
        Some("stopping")
    );
    assert!(matches!(
        store.bind_planner_pod(&c.id, session.generation, "resident-pod"),
        Err(CleanStoreError::StaleVersion)
    ));
    Ok(())
}

#[test]
fn accepted_message_after_idle_deadline_cannot_resurrect_an_expired_resident()
-> Result<(), CleanStoreError> {
    let (mut store, c) = setup_session()?;
    enqueue(&mut store, &c, "first", "1")?;
    let old = finish(&mut store, &c, "first")?;
    let now = unix_seconds()?;
    store.connection.execute("UPDATE project_planner_sessions SET payload=json_set(payload,'$.idle_started_at',?1,'$.idle_expires_at',?2) WHERE conversation_id=?3",params![now-301,now-1,c.id]).map_err(CleanStoreError::Database)?;
    enqueue(&mut store, &c, "after-expiry", "1")?;
    let expired = store
        .planner_session(&c.id)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(expired.state, "stopping");
    assert!(expired.pending_run_id.is_empty());
    assert!(
        store
            .prepare_planner_session(&c.id, "after-expiry", 600)?
            .is_none()
    );
    store.stop_planner_session(&c.id, old.generation, false)?;
    let (fresh, _) = store
        .prepare_planner_session(&c.id, "after-expiry", 600)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(fresh.generation, old.generation + 1);
    Ok(())
}
#[test]
fn timely_accepted_queued_turn_keeps_its_reservation_while_waiting_for_dispatch()
-> Result<(), CleanStoreError> {
    let (mut store, c) = setup_session()?;
    enqueue(&mut store, &c, "first", "1")?;
    let old = finish(&mut store, &c, "first")?;
    enqueue(&mut store, &c, "queued-before-expiry", "1")?;
    let queued = store
        .planner_session(&c.id)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(queued.pending_run_id, "queued-before-expiry");
    let now = unix_seconds()?;
    store.connection.execute("UPDATE project_planner_sessions SET payload=json_set(payload,'$.idle_started_at',?1,'$.idle_expires_at',?2) WHERE conversation_id=?3",params![now-1000,now-700,c.id]).map_err(CleanStoreError::Database)?;
    assert!(!store.expire_planner_session(&c.id, old.generation, now)?);
    let (same, _) = store
        .prepare_planner_session(&c.id, "queued-before-expiry", 600)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(same.session_id, old.session_id);
    assert_eq!(same.generation, old.generation);
    assert!(same.pending_run_id.is_empty());
    Ok(())
}
