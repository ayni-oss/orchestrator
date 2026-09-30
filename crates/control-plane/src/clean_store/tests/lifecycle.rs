use super::*;

#[test]
fn initializes_exact_clean_schema_and_reopens() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("orchestrator.sqlite");
    let store = CleanStore::open(&path, 3)?;
    assert_eq!(store.schema_version()?, 1);
    assert!(existing_tables(&store.connection)?.contains(&"runtime_profiles".to_owned()));
    assert!(!existing_tables(&store.connection)?.contains(&"repository_bindings".to_owned()));
    drop(store);
    let reopened = CleanStore::open(path, 3)?;
    reopened.integrity_check()?;
    Ok(())
}

fn rejects_retired_pilot_schema_without_mutating_it() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("orchestrator.sqlite");
    let connection = Connection::open(&path)?;
    connection.execute_batch("CREATE TABLE tasks(id TEXT PRIMARY KEY);")?;
    drop(connection);
    assert!(matches!(
        CleanStore::open(&path, 3),
        Err(CleanStoreError::RetiredPilotSchema)
    ));
    let connection = Connection::open(path)?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tasks'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(count, 1);
    Ok(())
}

fn guarded_reset_of_retired_volume_produces_exact_clean_baseline()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("orchestrator.sqlite");
    let connection = Connection::open(&path)?;
    connection.execute_batch(
        "CREATE TABLE tasks(id TEXT PRIMARY KEY); CREATE TABLE preparation_records(id TEXT);",
    )?;
    drop(connection);
    assert!(matches!(
        CleanStore::open(&path, 3),
        Err(CleanStoreError::RetiredPilotSchema)
    ));

    fs::remove_file(&path)?;
    let store = CleanStore::open(path, 3)?;
    assert_eq!(
        existing_tables(&store.connection)?,
        [
            "assessments",
            "automation_installations",
            "candidates",
            "change_requests",
            "clean_schema",
            "development_attempts",
            "development_tasks",
            "durable_actions",
            "event_watermarks",
            "evidence_references",
            "page_snapshots",
            "reconciliation_cursors",
            "repository_connections",
            "repository_snapshots",
            "resource_events",
            "runtime_artifacts",
            "runtime_profile_revisions",
            "runtime_profiles",
        ]
        .map(str::to_owned)
    );
    Ok(())
}
fn idempotent_create_replays_and_conflicting_content_fails() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let request = repository_request("public-key", "123");
    let first = store.create_repository_connection("scope", &request)?;
    let replay = store.create_repository_connection("scope", &request)?;
    assert_eq!(replay.id, first.id);
    let changed = repository_request("public-key", "456");
    assert!(matches!(
        store.create_repository_connection("scope", &changed),
        Err(CleanStoreError::IdempotencyConflict)
    ));
    Ok(())
}

#[test]
fn pages_retain_values_across_writes_and_restart() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("orchestrator.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let first = store.create_repository_connection("one", &repository_request("one", "1"))?;
    let second = store.create_repository_connection("two", &repository_request("two", "2"))?;
    let page = store.list_repository_connections(
        None,
        Some(&v1::PageRequest {
            page_size: 1,
            page_token: String::new(),
        }),
    )?;
    assert_eq!(page.items.len(), 1);
    assert!(!page.next_page_token.is_empty());
    let expected_remaining = if page.items[0].id == first.id {
        second.id
    } else {
        first.id
    };
    let token = page.next_page_token;
    let mut tampered = token.clone().into_bytes();
    let last = tampered
        .last_mut()
        .ok_or(CleanStoreError::InvalidPageToken)?;
    *last = if *last == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(tampered)?;
    assert!(matches!(
        store.list_repository_connections(
            None,
            Some(&v1::PageRequest {
                page_size: 1,
                page_token: tampered,
            }),
        ),
        Err(CleanStoreError::InvalidPageToken)
    ));
    store.create_repository_connection("three", &repository_request("three", "3"))?;
    drop(store);
    let mut reopened = CleanStore::open(path, 3)?;
    let next = reopened.list_repository_connections(
        None,
        Some(&v1::PageRequest {
            page_size: 1,
            page_token: token,
        }),
    )?;
    assert_eq!(next.items[0].id, expected_remaining);
    assert!(next.next_page_token.is_empty());
    Ok(())
}

#[test]
fn pagination_rejects_unbounded_snapshots_and_responses() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let oversized = vec![vec![0_u8; MAX_PAGE_SNAPSHOT_BYTES.saturating_add(1)]];
    assert!(matches!(
        store.materialize_page::<v1::RepositoryConnection>(
            "ListRepositoryConnections",
            &encode_filter(&[""]),
            0,
            oversized,
            0,
            1,
        ),
        Err(CleanStoreError::BoundExceeded)
    ));
    store
        .connection
        .execute_batch(
            "CREATE TEMP TABLE oversized_payloads(payload BLOB NOT NULL);
             INSERT INTO oversized_payloads VALUES (zeroblob(5000000)),(zeroblob(5000000));",
        )
        .map_err(CleanStoreError::Database)?;
    let mut statement = store
        .connection
        .prepare("SELECT payload FROM oversized_payloads")
        .map_err(CleanStoreError::Database)?;
    let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
    assert!(matches!(
        collect_bounded_payloads(&mut rows),
        Err(CleanStoreError::BoundExceeded)
    ));
    Ok(())
}

#[test]
fn external_action_recovers_at_every_durable_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("orchestrator.sqlite");
    let request = v1::ReconcileRuntimeProfileRequest {
        idempotency_key: "reconcile".to_owned(),
        runtime_profile_id: "profile:missing".to_owned(),
    };

    let mut store = CleanStore::open(&path, 3)?;
    let pending = store.begin_external_action(
        "ReconcileRuntimeProfile",
        "scoped-key",
        &request,
        "runtime_profile",
        "profile:missing",
    )?;
    assert_eq!(pending.state, DurableActionState::Pending);
    drop(store);

    let mut store = CleanStore::open(&path, 3)?;
    let recovered = store.begin_external_action(
        "ReconcileRuntimeProfile",
        "scoped-key",
        &request,
        "runtime_profile",
        "profile:missing",
    )?;
    assert_eq!(recovered.state, DurableActionState::Pending);
    assert_eq!(
        store.connection.query_row(
            "SELECT COUNT(*) FROM durable_actions WHERE operation='ReconcileRuntimeProfile'",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        1
    );
    store.fail_external_action("ReconcileRuntimeProfile", "scoped-key", &"é".repeat(600))?;
    drop(store);

    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(
        store
            .durable_action("ReconcileRuntimeProfile", "scoped-key")?
            .map(|value| value.state),
        Some(DurableActionState::Failed)
    );
    assert!(matches!(
        store.complete_external_action("ReconcileRuntimeProfile", "scoped-key"),
        Err(CleanStoreError::InvalidTransition)
    ));
    store.requeue_external_action("ReconcileRuntimeProfile", "scoped-key")?;
    drop(store);

    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(
        store
            .pending_external_actions("ReconcileRuntimeProfile")?
            .len(),
        1
    );
    store.complete_external_action("ReconcileRuntimeProfile", "scoped-key")?;
    drop(store);

    let mut store = CleanStore::open(path, 3)?;
    let completed = store.begin_external_action(
        "ReconcileRuntimeProfile",
        "scoped-key",
        &request,
        "runtime_profile",
        "profile:missing",
    )?;
    assert_eq!(completed.state, DurableActionState::Completed);
    assert_eq!(
        v1::ReconcileRuntimeProfileRequest::decode(completed.request_payload.as_slice(),)?,
        request
    );
    let conflicting = v1::ReconcileRuntimeProfileRequest {
        runtime_profile_id: "profile:different".to_owned(),
        ..request
    };
    assert!(matches!(
        store.begin_external_action(
            "ReconcileRuntimeProfile",
            "scoped-key",
            &conflicting,
            "runtime_profile",
            "profile:different",
        ),
        Err(CleanStoreError::IdempotencyConflict)
    ));
    Ok(())
}

#[test]
fn duplicate_and_late_artifact_reports_cannot_roll_back_active_revision()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, first_artifact) = ready_profile(&mut store)?;
    let duplicate = store.record_runtime_artifact(
        &profile.id,
        verified_artifact(
            &profile
                .active_revision_id
                .clone()
                .ok_or(CleanStoreError::CorruptState)?,
            DIGEST,
        ),
    )?;
    assert_eq!(duplicate.id, first_artifact.id);
    let competing = store.record_runtime_artifact(
        &profile.id,
        verified_artifact(
            &profile
                .active_revision_id
                .clone()
                .ok_or(CleanStoreError::CorruptState)?,
            "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        ),
    )?;
    assert_ne!(competing.id, first_artifact.id);
    assert_eq!(
        store
            .runtime_profile(&profile.id)?
            .active_runtime_artifact_id,
        Some(first_artifact.id)
    );

    let first_revision_id = profile
        .active_revision_id
        .clone()
        .ok_or(CleanStoreError::CorruptState)?;
    let configuration = store
        .runtime_profile_revision(&first_revision_id)?
        .configuration
        .ok_or(CleanStoreError::CorruptState)?;
    let snapshot = store.observe_repository_snapshot(&repository.id, SECOND_REVISION)?;
    let second_revision =
        store.record_runtime_profile_revision(&profile.id, &snapshot.id, configuration)?;
    let late_digest = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    store.record_runtime_artifact(
        &profile.id,
        verified_artifact(&first_revision_id, late_digest),
    )?;
    let retained = store.runtime_profile(&profile.id)?;
    assert_eq!(retained.active_revision_id, Some(second_revision.id));
    assert_eq!(retained.state, v1::RuntimeProfileState::Updating as i32);
    assert!(retained.active_runtime_artifact_id.is_none());
    Ok(())
}

#[test]
fn staged_artifact_verification_and_loss_update_profile_binding() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let repository =
        store.create_repository_connection("repo-scope", &repository_request("repo", "123"))?;
    let snapshot = store.observe_repository_snapshot(&repository.id, REVISION)?;
    let (profile, revision) = store.create_runtime_profile(
        "profile-scope",
        &profile_request("profile", &repository.id, &snapshot.id),
    )?;
    let mut report = verified_artifact(&revision.id, DIGEST);
    report.status = v1::RuntimeArtifactStatus::Discovered as i32;
    report.verified_at = None;
    let discovered = store.record_runtime_artifact(&profile.id, report)?;
    let verifying = store.transition_runtime_artifact(
        &discovered.id,
        discovered.resource_version,
        v1::RuntimeArtifactStatus::Verifying,
        None,
    )?;
    let verified = store.transition_runtime_artifact(
        &verifying.id,
        verifying.resource_version,
        v1::RuntimeArtifactStatus::Verified,
        None,
    )?;
    let ready = store.runtime_profile(&profile.id)?;
    assert_eq!(ready.active_runtime_artifact_id, Some(verified.id.clone()));
    assert_eq!(ready.state, v1::RuntimeProfileState::Ready as i32);
    let unavailable = store.transition_runtime_artifact(
        &verified.id,
        verified.resource_version,
        v1::RuntimeArtifactStatus::Unavailable,
        Some(v1::ResourceFailure {
            reason: v1::ErrorReason::RuntimeArtifactNotVerified as i32,
            message: "artifact is no longer available".to_owned(),
            remediation: None,
            observed_at: None,
        }),
    )?;
    assert_eq!(
        unavailable.status,
        v1::RuntimeArtifactStatus::Unavailable as i32
    );
    let degraded = store.runtime_profile(&profile.id)?;
    assert!(degraded.active_runtime_artifact_id.is_none());
    assert_eq!(degraded.state, v1::RuntimeProfileState::Degraded as i32);
    assert!(degraded.failure.is_some());

    let replacement = store
        .record_runtime_artifact(&profile.id, verified_artifact(&revision.id, SECOND_DIGEST))?;
    let restored = store.runtime_profile(&profile.id)?;
    assert_eq!(restored.active_runtime_artifact_id, Some(replacement.id));
    assert_eq!(restored.state, v1::RuntimeProfileState::Ready as i32);
    assert!(restored.failure.is_none());
    Ok(())
}

#[test]
fn task_freezes_runtime_and_rejects_stale_or_invalid_transitions() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, first_artifact) = ready_profile(&mut store)?;
    let (task, attempt) = store.create_development_task(
        "task-scope",
        &task_request(&repository.id, &profile.id, "task"),
    )?;
    let frozen = task
        .snapshot
        .as_ref()
        .and_then(|value| value.runtime.as_ref())
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(frozen.runtime_artifact_id, first_artifact.id);
    let revision_id = profile
        .active_revision_id
        .clone()
        .ok_or(CleanStoreError::CorruptState)?;
    let configuration = store
        .runtime_profile_revision(&revision_id)?
        .configuration
        .ok_or(CleanStoreError::CorruptState)?;
    let second_snapshot = store.observe_repository_snapshot(&repository.id, SECOND_REVISION)?;
    let second_revision =
        store.record_runtime_profile_revision(&profile.id, &second_snapshot.id, configuration)?;
    let mut waiting_request = task_request(&repository.id, &profile.id, "waiting-task");
    if let Some(request) = waiting_request.request.as_mut() {
        request.repository_revision = SECOND_REVISION.to_owned();
    }
    assert!(matches!(
        store.create_development_task("waiting-task-scope", &waiting_request),
        Err(CleanStoreError::ProfileNotReady)
    ));
    let second = store.record_runtime_artifact(
        &profile.id,
        verified_artifact(&second_revision.id, SECOND_DIGEST),
    )?;
    assert_ne!(second.id, first_artifact.id);
    let (new_task, _) = store.create_development_task("new-task-scope", &waiting_request)?;
    assert_eq!(
        new_task
            .snapshot
            .and_then(|value| value.runtime)
            .map(|value| value.runtime_artifact_id),
        Some(second.id)
    );
    let retained = store.development_task(&task.id)?;
    assert_eq!(
        retained
            .snapshot
            .and_then(|value| value.runtime)
            .map(|value| value.runtime_artifact_id),
        Some(first_artifact.id)
    );
    assert!(matches!(
        store.transition_attempt(
            &attempt.id,
            attempt.resource_version,
            v1::DevelopmentAttemptState::Succeeded,
            None,
        ),
        Err(CleanStoreError::InvalidTransition)
    ));
    let running = store.transition_attempt(
        &attempt.id,
        attempt.resource_version,
        v1::DevelopmentAttemptState::Running,
        None,
    )?;
    let duplicate = store.transition_attempt(
        &attempt.id,
        attempt.resource_version,
        v1::DevelopmentAttemptState::Running,
        None,
    )?;
    assert_eq!(duplicate.resource_version, running.resource_version);
    assert!(matches!(
        store.transition_attempt(
            &attempt.id,
            attempt.resource_version,
            v1::DevelopmentAttemptState::Failed,
            None,
        ),
        Err(CleanStoreError::StaleVersion)
    ));
    let succeeded = store.transition_attempt(
        &attempt.id,
        running.resource_version,
        v1::DevelopmentAttemptState::Succeeded,
        None,
    )?;
    assert_eq!(
        succeeded.state,
        v1::DevelopmentAttemptState::Succeeded as i32
    );
    assert!(matches!(
        store.transition_attempt(
            &attempt.id,
            succeeded.resource_version,
            v1::DevelopmentAttemptState::Failed,
            None,
        ),
        Err(CleanStoreError::InvalidTransition)
    ));
    Ok(())
}

#[test]
fn cancellation_is_terminal_against_late_reports() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (task, attempt) = store.create_development_task(
        "task-scope",
        &task_request(&repository.id, &profile.id, "task"),
    )?;
    let cancelled = store.cancel_development_task(
        "cancel-scope",
        &v1::CancelDevelopmentTaskRequest {
            idempotency_key: "cancel".to_owned(),
            development_task_id: task.id.clone(),
        },
    )?;
    assert_eq!(cancelled.state, v1::DevelopmentTaskState::Cancelled as i32);
    store.cancel_development_task(
        "second-cancel-scope",
        &v1::CancelDevelopmentTaskRequest {
            idempotency_key: "second-cancel".to_owned(),
            development_task_id: task.id,
        },
    )?;
    let (second_task, _) = store.create_development_task(
        "second-task-scope",
        &task_request(&repository.id, &profile.id, "second-task"),
    )?;
    assert!(matches!(
        store.cancel_development_task(
            "second-cancel-scope",
            &v1::CancelDevelopmentTaskRequest {
                idempotency_key: "second-cancel".to_owned(),
                development_task_id: second_task.id,
            },
        ),
        Err(CleanStoreError::IdempotencyConflict)
    ));
    let cancelled_attempt = store.development_attempt(&attempt.id)?;
    assert!(matches!(
        store.transition_attempt(
            &attempt.id,
            cancelled_attempt.resource_version,
            v1::DevelopmentAttemptState::Succeeded,
            None,
        ),
        Err(CleanStoreError::InvalidTransition)
    ));
    Ok(())
}

#[test]
fn clean_marker_does_not_hide_retired_tables() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("orchestrator.sqlite");
    drop(CleanStore::open(&path, 3)?);
    let connection = Connection::open(&path)?;
    connection.execute_batch("CREATE TABLE tasks(id TEXT PRIMARY KEY);")?;
    drop(connection);
    assert!(matches!(
        CleanStore::open(&path, 3),
        Err(CleanStoreError::RetiredPilotSchema)
    ));
    let connection = Connection::open(path)?;
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tasks'",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        1
    );
    Ok(())
}

#[test]
fn create_profile_replay_keeps_the_original_revision() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let repository =
        store.create_repository_connection("repo-scope", &repository_request("repo", "123"))?;
    let snapshot = store.observe_repository_snapshot(&repository.id, REVISION)?;
    let request = profile_request("profile", &repository.id, &snapshot.id);
    let (profile, initial_revision) = store.create_runtime_profile("profile-scope", &request)?;
    let configuration = initial_revision
        .configuration
        .clone()
        .ok_or(CleanStoreError::CorruptState)?;
    let next_snapshot = store.observe_repository_snapshot(&repository.id, SECOND_REVISION)?;
    let next_revision = store.record_runtime_profile_revision(
        &profile.id,
        &next_snapshot.id,
        configuration.clone(),
    )?;
    let stale_duplicate =
        store.record_runtime_profile_revision(&profile.id, &snapshot.id, configuration)?;
    assert_eq!(stale_duplicate.id, initial_revision.id);
    assert_eq!(
        store.runtime_profile(&profile.id)?.active_revision_id,
        Some(next_revision.id.clone())
    );
    let (replayed_profile, replayed_revision) =
        store.create_runtime_profile("profile-scope", &request)?;
    assert_eq!(replayed_revision.id, initial_revision.id);
    assert_eq!(replayed_revision.revision_number, 1);
    assert_eq!(replayed_profile.active_revision_id, Some(next_revision.id));
    Ok(())
}

#[test]
fn create_task_replay_keeps_the_original_initial_attempt() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let request = task_request(&repository.id, &profile.id, "task");
    let (task, initial) = store.create_development_task("task-scope", &request)?;
    let running = store.transition_attempt(
        &initial.id,
        initial.resource_version,
        v1::DevelopmentAttemptState::Running,
        None,
    )?;
    store.transition_attempt(
        &initial.id,
        running.resource_version,
        v1::DevelopmentAttemptState::Failed,
        None,
    )?;
    let retry = store.retry_development_task(
        "retry-scope",
        &v1::RetryDevelopmentTaskRequest {
            idempotency_key: "retry".to_owned(),
            development_task_id: task.id,
        },
    )?;
    assert_eq!(retry.attempt_number, 2);
    let (replayed_task, replayed_initial) =
        store.create_development_task("task-scope", &request)?;
    assert_eq!(replayed_initial.id, initial.id);
    assert_eq!(replayed_initial.attempt_number, 1);
    assert_eq!(replayed_task.active_attempt_id, Some(retry.id));
    Ok(())
}
