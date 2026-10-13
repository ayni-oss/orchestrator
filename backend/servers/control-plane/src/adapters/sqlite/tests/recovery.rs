use super::*;

#[test]
fn semantically_empty_or_index_mismatched_payload_fails_closed()
-> Result<(), Box<dyn std::error::Error>> {
    for corruption in ["payload=x''", "status=999"] {
        let directory = tempdir()?;
        let path = directory.path().join("orchestrator.sqlite");
        let mut store = CleanStore::open(&path, 3)?;
        let repository =
            store.create_repository_connection("repo-scope", &repository_request("repo", "123"))?;
        store.connection.execute(
            &format!("UPDATE repository_connections SET {corruption} WHERE id=?1"),
            [&repository.id],
        )?;
        drop(store);
        assert!(matches!(
            CleanStore::open(path, 3),
            Err(CleanStoreError::CorruptState)
        ));
    }
    Ok(())
}

#[test]
fn corrupt_payload_fails_closed_on_read_and_restart() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("orchestrator.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let repository =
        store.create_repository_connection("repo-scope", &repository_request("repo", "123"))?;
    store.connection.execute(
        "UPDATE repository_connections SET payload=x'00' WHERE id=?1",
        [&repository.id],
    )?;
    assert!(matches!(
        store.repository_connection(&repository.id),
        Err(CleanStoreError::CorruptState)
    ));
    drop(store);
    assert!(matches!(
        CleanStore::open(path, 3),
        Err(CleanStoreError::CorruptState)
    ));
    Ok(())
}

#[test]
fn corrupt_event_or_page_snapshot_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    for corruption in ["event", "page"] {
        let directory = tempdir()?;
        let path = directory.path().join("orchestrator.sqlite");
        let mut store = CleanStore::open(&path, 3)?;
        store.create_repository_connection("one", &repository_request("one", "1"))?;
        if corruption == "event" {
            store.connection.execute(
                    "UPDATE resource_events SET payload=x'' WHERE cursor=(SELECT MIN(cursor) FROM resource_events)",
                    [],
                )?;
        } else {
            store.create_repository_connection("two", &repository_request("two", "2"))?;
            let page = store.list_repository_connections(
                None,
                Some(&v1::PageRequest {
                    page_size: 1,
                    page_token: String::new(),
                }),
            )?;
            assert!(!page.next_page_token.is_empty());
            store
                .connection
                .execute("UPDATE page_snapshots SET items='[\"not-base64!\"]'", [])?;
        }
        drop(store);
        assert!(matches!(
            CleanStore::open(path, 3),
            Err(CleanStoreError::CorruptState)
        ));
    }
    Ok(())
}

#[test]
fn fractional_deadlines_preserve_subsecond_precision() -> Result<(), CleanStoreError> {
    let base = Timestamp {
        seconds: 10,
        nanos: 900_000_000,
    };
    let deadline = add_duration(
        &base,
        &prost_types::Duration {
            seconds: 0,
            nanos: 200_000_000,
        },
    )?;
    assert_eq!(
        deadline,
        Timestamp {
            seconds: 11,
            nanos: 100_000_000,
        }
    );
    Ok(())
}

#[test]
fn reconciliation_expires_task_and_attempt_deadlines() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (task, attempt) = store.create_development_task(
        "task-scope",
        &task_request(&store, &repository.id, &profile.id, "task")?,
    )?;
    assert_eq!(store.expire_deadlines(&timestamp(i64::MAX / 2))?, 1);
    assert_eq!(
        store.development_task(&task.id)?.state,
        v1::DevelopmentTaskState::DeadlineExceeded as i32
    );
    assert_eq!(
        store.development_attempt(&attempt.id)?.state,
        v1::DevelopmentAttemptState::DeadlineExceeded as i32
    );
    assert_eq!(store.expire_deadlines(&timestamp(i64::MAX / 2))?, 0);
    Ok(())
}

#[test]
fn transition_tables_reject_every_terminal_source() {
    use v1::DevelopmentAttemptState as State;
    for terminal in [
        State::Succeeded,
        State::Failed,
        State::Cancelled,
        State::DeadlineExceeded,
        State::Unsupported,
    ] {
        for target in [
            State::Queued,
            State::Running,
            State::WaitingForInput,
            State::Succeeded,
            State::Failed,
            State::Cancelled,
            State::DeadlineExceeded,
            State::Unsupported,
        ] {
            assert!(!valid_attempt_transition(terminal, target));
        }
    }
}

#[test]
fn resource_transition_tables_match_only_supported_edges() {
    use v1::{
        RepositoryConnectionStatus as Repository, RuntimeArtifactStatus as Artifact,
        RuntimeProfileState as Profile,
    };
    let repository_states = [
        Repository::Connecting,
        Repository::Connected,
        Repository::Degraded,
        Repository::Revoked,
        Repository::Failed,
    ];
    let repository_edges = [
        (Repository::Connecting, Repository::Connected),
        (Repository::Connecting, Repository::Degraded),
        (Repository::Connecting, Repository::Revoked),
        (Repository::Connecting, Repository::Failed),
        (Repository::Connected, Repository::Degraded),
        (Repository::Connected, Repository::Revoked),
        (Repository::Degraded, Repository::Connecting),
        (Repository::Degraded, Repository::Connected),
        (Repository::Degraded, Repository::Revoked),
        (Repository::Degraded, Repository::Failed),
    ];
    for current in repository_states {
        for next in repository_states {
            assert_eq!(
                valid_repository_transition(current, next),
                repository_edges.contains(&(current, next)),
                "unexpected repository transition {current:?} -> {next:?}"
            );
        }
    }

    let profile_states = [
        Profile::SetupRequired,
        Profile::SetupPrOpen,
        Profile::AwaitingEnvironment,
        Profile::Ready,
        Profile::Updating,
        Profile::Degraded,
        Profile::Failed,
    ];
    for current in profile_states {
        for next in profile_states {
            let expected = match current {
                Profile::SetupRequired => matches!(
                    next,
                    Profile::SetupPrOpen
                        | Profile::AwaitingEnvironment
                        | Profile::Degraded
                        | Profile::Failed
                ),
                Profile::SetupPrOpen => matches!(
                    next,
                    Profile::SetupRequired
                        | Profile::AwaitingEnvironment
                        | Profile::Degraded
                        | Profile::Failed
                ),
                Profile::AwaitingEnvironment => matches!(
                    next,
                    Profile::SetupRequired
                        | Profile::Ready
                        | Profile::Updating
                        | Profile::Degraded
                        | Profile::Failed
                ),
                Profile::Ready => matches!(
                    next,
                    Profile::SetupRequired | Profile::Updating | Profile::Degraded
                ),
                Profile::Updating => {
                    matches!(
                        next,
                        Profile::SetupRequired
                            | Profile::Ready
                            | Profile::Degraded
                            | Profile::Failed
                    )
                }
                Profile::Degraded => matches!(
                    next,
                    Profile::SetupRequired
                        | Profile::AwaitingEnvironment
                        | Profile::Updating
                        | Profile::Ready
                        | Profile::Failed
                ),
                Profile::Failed | Profile::Unspecified => false,
            };
            assert_eq!(valid_profile_transition(current, next), expected);
        }
    }

    let artifact_states = [
        Artifact::Discovered,
        Artifact::Verifying,
        Artifact::Verified,
        Artifact::Rejected,
        Artifact::Unavailable,
        Artifact::Failed,
    ];
    for current in artifact_states {
        for next in artifact_states {
            let expected = match current {
                Artifact::Discovered => matches!(
                    next,
                    Artifact::Verifying
                        | Artifact::Rejected
                        | Artifact::Unavailable
                        | Artifact::Failed
                ),
                Artifact::Verifying => matches!(
                    next,
                    Artifact::Verified
                        | Artifact::Rejected
                        | Artifact::Unavailable
                        | Artifact::Failed
                ),
                Artifact::Verified => next == Artifact::Unavailable,
                Artifact::Unavailable => matches!(next, Artifact::Verifying | Artifact::Failed),
                Artifact::Rejected | Artifact::Failed | Artifact::Unspecified => false,
            };
            assert_eq!(valid_artifact_transition(current, next), expected);
        }
    }
}

#[test]
fn watch_rejects_pruned_and_future_cursors() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let transaction = store
        .connection
        .transaction()
        .map_err(CleanStoreError::Database)?;
    for version in 1..=MAX_EVENTS_PER_SERVICE.saturating_add(2) {
        let resource = v1::RepositoryConnection {
            id: format!("repository-{version}"),
            resource_version: u64::try_from(version).map_err(|_| CleanStoreError::CorruptState)?,
            ..v1::RepositoryConnection::default()
        };
        append_event(
            &transaction,
            REPOSITORY_SERVICE,
            "repository_connection",
            &resource.id,
            resource.resource_version,
            Some(&resource.id),
            None,
            &resource,
        )?;
    }
    transaction.commit().map_err(CleanStoreError::Database)?;
    let watermark: i64 = store
        .connection
        .query_row(
            "SELECT first_cursor FROM event_watermarks WHERE service=?1",
            [REPOSITORY_SERVICE],
            |row| row.get(0),
        )
        .map_err(CleanStoreError::Database)?;
    assert!(watermark > 1);
    assert!(matches!(
        store.events_after(REPOSITORY_SERVICE, 0, None, None),
        Err(CleanStoreError::WatchCursorExpired)
    ));
    let high_water = store.current_cursor(REPOSITORY_SERVICE)?;
    assert!(matches!(
        store.events_after(REPOSITORY_SERVICE, high_water.saturating_add(1), None, None,),
        Err(CleanStoreError::WatchCursorExpired)
    ));
    assert!(
        !store
            .events_after(
                REPOSITORY_SERVICE,
                u64::try_from(watermark.saturating_sub(1))
                    .map_err(|_| CleanStoreError::CorruptState)?,
                None,
                None,
            )?
            .is_empty()
    );
    Ok(())
}

#[test]
fn service_events_are_monotonic_and_resumable() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let first = store.create_repository_connection("one", &repository_request("one", "1"))?;
    let events = store.events_after(REPOSITORY_SERVICE, 0, Some(&first.id), None)?;
    assert_eq!(events.len(), 1);
    let cursor = events[0].cursor;
    store.observe_repository_snapshot(&first.id, SECOND_REVISION)?;
    let resumed = store.events_after(REPOSITORY_SERVICE, cursor, Some(&first.id), None)?;
    assert_eq!(resumed.len(), 1);
    assert!(resumed[0].cursor > cursor);
    store
        .connection
        .execute(
            "DELETE FROM resource_events WHERE service=?1",
            [REPOSITORY_SERVICE],
        )
        .map_err(CleanStoreError::Database)?;
    let (current, high_water) = store.initial_events(REPOSITORY_SERVICE, Some(&first.id), None)?;
    assert_eq!(current.len(), 2);
    assert_eq!(high_water, 0);
    Ok(())
}

#[test]
fn deadline_index_and_atomic_events_respect_the_exact_frozen_timestamp()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (task, attempt) = store.create_development_task(
        "precise-expiry",
        &task_request(&store, &repository.id, &profile.id, "precise-expiry")?,
    )?;
    let deadline = task
        .snapshot
        .as_ref()
        .and_then(|s| s.deadline_at.as_ref())
        .ok_or("deadline")?;
    let just_before = if deadline.nanos == 0 {
        Timestamp {
            seconds: deadline.seconds - 1,
            nanos: 999_999_999,
        }
    } else {
        Timestamp {
            seconds: deadline.seconds,
            nanos: deadline.nanos - 1,
        }
    };
    let before: i64 =
        store
            .connection
            .query_row("SELECT COUNT(*) FROM resource_events", [], |row| row.get(0))?;
    assert_eq!(store.expire_deadlines(&just_before)?, 0);
    assert_eq!(store.expire_deadlines(deadline)?, 1);
    let after: i64 =
        store
            .connection
            .query_row("SELECT COUNT(*) FROM resource_events", [], |row| row.get(0))?;
    assert_eq!(after, before + 2);
    let completed = store.development_attempt(&attempt.id)?;
    assert_eq!(
        completed.state,
        v1::DevelopmentAttemptState::DeadlineExceeded as i32
    );
    assert_eq!(completed.completed_at.as_ref(), Some(deadline));
    assert_eq!(store.expire_deadlines(deadline)?, 0);
    assert_eq!(store.development_attempt(&attempt.id)?, completed);
    let replay: i64 =
        store
            .connection
            .query_row("SELECT COUNT(*) FROM resource_events", [], |row| row.get(0))?;
    assert_eq!(replay, after);
    Ok(())
}

#[test]
fn expiry_preserves_a_terminal_active_attempt_and_only_appends_the_task_event()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (task, mut attempt) = store.create_development_task(
        "terminal-expiry",
        &task_request(&store, &repository.id, &profile.id, "terminal-expiry")?,
    )?;
    // Retained terminal evidence may precede the task's deadline reconciliation.
    attempt.state = v1::DevelopmentAttemptState::Succeeded as i32;
    attempt.completed_at = Some(timestamp(1));
    store.connection.execute(
        "UPDATE development_attempts SET state=?1,payload=?2 WHERE id=?3",
        params![attempt.state, attempt.encode_to_vec(), attempt.id],
    )?;
    let before: i64 =
        store
            .connection
            .query_row("SELECT COUNT(*) FROM resource_events", [], |row| row.get(0))?;
    assert_eq!(store.expire_deadlines(&timestamp(i64::MAX / 2))?, 1);
    assert_eq!(
        store.development_task(&task.id)?.state,
        v1::DevelopmentTaskState::DeadlineExceeded as i32
    );
    assert_eq!(store.development_attempt(&attempt.id)?, attempt);
    let after: i64 =
        store
            .connection
            .query_row("SELECT COUNT(*) FROM resource_events", [], |row| row.get(0))?;
    assert_eq!(after, before + 1);
    Ok(())
}

#[test]
fn collects_worker_result_once_and_retries_only_retained_publication() -> Result<(), CleanStoreError>
{
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (task, attempt) = store.create_development_task(
        "worker-task",
        &task_request(&store, &repository.id, &profile.id, "worker-task")?,
    )?;
    let result = v1::WorkerResult {
        execution: v1::ExecutionOutcome::Succeeded as i32,
        commit: Some(SECOND_REVISION.into()),
        changed_paths: vec!["README.md".into()],
        quality: Some(v1::QualityResult {
            outcome: v1::QualityOutcome::Unavailable as i32,
            findings: vec!["Check runner unavailable".into()],
            ..Default::default()
        }),
        publication: v1::PublicationOutcome::Failed as i32,
        coding_turns_consumed: 5,
        quality_round: 1,
        ..Default::default()
    };
    let collected = store.collect_worker(&attempt.id, None, Some(result.clone()))?;
    assert_eq!(
        collected.state,
        v1::DevelopmentAttemptState::Succeeded as i32
    );
    assert_eq!(
        store
            .collect_worker(&attempt.id, None, Some(result))?
            .resource_version,
        collected.resource_version
    );
    let request = v1::RetryTaskPublicationRequest {
        idempotency_key: "publication-retry".into(),
        development_task_id: task.id,
    };
    let retried = store.retry_task_publication("publication-retry", &request)?;
    assert!(retried.publication_only);
    assert_eq!(retried.id, attempt.id);
    assert_eq!(
        retried.result.as_ref().and_then(|r| r.commit.as_deref()),
        Some(SECOND_REVISION)
    );
    assert_eq!(
        store
            .retry_task_publication("publication-retry", &request)?
            .resource_version,
        retried.resource_version
    );
    Ok(())
}

#[test]
fn passing_quality_requires_bound_full_check_evidence() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (_, attempt) = store.create_development_task(
        "quality-task",
        &task_request(&store, &repository.id, &profile.id, "quality-task")?,
    )?;
    let result = v1::WorkerResult {
        execution: v1::ExecutionOutcome::Succeeded as i32,
        commit: Some(SECOND_REVISION.into()),
        quality: Some(v1::QualityResult {
            outcome: v1::QualityOutcome::Passed as i32,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(matches!(
        store.collect_worker(&attempt.id, None, Some(result)),
        Err(CleanStoreError::CorruptState)
    ));
    assert!(store.development_attempt(&attempt.id)?.result.is_none());
    Ok(())
}

#[test]
fn schema_seven_cutover_retires_image_automation_and_preserves_frozen_work()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("cutover.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (task, attempt) = store.create_development_task(
        "task",
        &task_request(&store, &repository.id, &profile.id, "task")?,
    )?;
    store.connection.execute_batch("DROP TABLE lifecycle_outbox; DROP TABLE lifecycle_journal; DROP TABLE coding_inbox; DROP TABLE coding_executions; DROP TABLE runtime_operation_inbox; DROP TABLE verification_consumer; DROP TABLE verification_outbox; DROP TABLE verification_journal; DROP TABLE verification_inbox; CREATE TABLE automation_installations(id TEXT PRIMARY KEY); DROP INDEX development_tasks_project_created_idx; ALTER TABLE development_tasks DROP COLUMN created_at_seconds; ALTER TABLE development_tasks DROP COLUMN created_at_nanos; UPDATE clean_schema SET version=7 WHERE component='control_plane';")?;
    store.connection.execute(
        "INSERT INTO automation_installations(id) VALUES('obsolete')",
        [],
    )?;
    store.connection.execute("INSERT INTO durable_actions(operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at) VALUES('InstallRepositoryAutomation','obsolete',x'00',x'00','pending','automation_installation','obsolete',1,1)", [])?;
    store.connection.execute("INSERT INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES(?1,?2,1,1)", params![format!("managed:{}",profile.id),r#"{"sequence":4,"next_at":0,"failures":3,"pending":{"old":"combined-build-push"},"bindings":{},"status":{},"last_success":null}"#])?;
    drop(store);
    let store = CleanStore::open(&path, 3)?;
    assert_eq!(
        store.schema_version()?,
        crate::adapters::sqlite::SCHEMA_VERSION
    );
    assert_eq!(store.repository_connection(&repository.id)?, repository);
    assert_eq!(store.development_task(&task.id)?.snapshot, task.snapshot);
    assert_eq!(
        store.development_attempt(&attempt.id)?.snapshot,
        attempt.snapshot
    );
    assert!(
        store
            .pending_external_actions("InstallRepositoryAutomation")?
            .is_empty()
    );
    let cursor = serde_json::to_value(store.runtime_cursor(&profile.id)?)?;
    assert!(cursor["pending"].is_null());
    assert_eq!(cursor["sequence"], 4);
    assert_eq!(cursor["failures"], 0);
    assert!(
        !existing_tables(&store.connection)?
            .iter()
            .any(|name| name == "automation_installations")
    );
    Ok(())
}

#[test]
fn operator_stream_snapshot_covers_resources_created_before_event_history()
-> Result<(), Box<dyn std::error::Error>> {
    let store = CleanStore::in_memory()?;
    let connection = v1::Connection {
        id: "legacy".into(),
        kind: "ghcr".into(),
        management_source: "ui".into(),
        resource_version: 1,
        ..Default::default()
    };
    store.connection.execute(
        "INSERT INTO connections(id,kind,payload) VALUES (?1,?2,?3)",
        params![connection.id, connection.kind, connection.encode_to_vec()],
    )?;
    let (events, high_water) = store.initial_events(CONNECTION_SERVICE, None, None)?;
    assert_eq!(high_water, 0);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cursor, 0);
    assert_eq!(
        v1::Connection::decode(events[0].payload.as_slice())?,
        connection
    );
    assert!(
        store
            .events_after(CONNECTION_SERVICE, 0, None, None)?
            .is_empty()
    );
    assert!(matches!(
        store.events_after(CONNECTION_SERVICE, 1, None, None),
        Err(CleanStoreError::WatchCursorExpired)
    ));
    Ok(())
}

#[test]
fn project_stream_snapshot_contains_current_membership() -> Result<(), Box<dyn std::error::Error>> {
    let mut store = CleanStore::in_memory()?;
    let repository =
        store.create_repository_connection("stream-repo", &repository_request("repo", "123"))?;
    let project = store.create_project(
        "stream-project",
        &v1::CreateProjectRequest {
            idempotency_key: "stream-project".into(),
            display_name: "Project".into(),
            repository_connection_ids: vec![repository.id],
            ..Default::default()
        },
    )?;
    let (events, cursor) = store.initial_events(PROJECT_SERVICE, None, None)?;
    assert_eq!(events.len(), 1);
    assert_eq!(v1::Project::decode(events[0].payload.as_slice())?, project);
    assert_eq!(cursor, events[0].cursor);
    assert!(
        store
            .events_after(PROJECT_SERVICE, cursor, None, None)?
            .is_empty()
    );
    Ok(())
}

#[test]
fn stream_snapshots_reject_oversized_resources_without_partial_success()
-> Result<(), Box<dyn std::error::Error>> {
    let store = CleanStore::in_memory()?;
    let connection = v1::Connection {
        id: "oversized".into(),
        kind: "ghcr".into(),
        management_source: "ui".into(),
        resource_version: 1,
        display_name: "x".repeat(MAX_PAGE_SNAPSHOT_BYTES + 1),
        ..Default::default()
    };
    store.connection.execute(
        "INSERT INTO connections(id,kind,payload) VALUES (?1,?2,?3)",
        params![connection.id, connection.kind, connection.encode_to_vec()],
    )?;
    assert!(matches!(
        store.initial_events(CONNECTION_SERVICE, None, None),
        Err(CleanStoreError::BoundExceeded)
    ));
    Ok(())
}
