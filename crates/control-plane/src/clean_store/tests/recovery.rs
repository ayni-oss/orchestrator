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
fn publication_action_retains_intent_and_requeues_after_observation() -> Result<(), CleanStoreError>
{
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (task, attempt) = store.create_development_task(
        "task-scope",
        &task_request(&repository.id, &profile.id, "task"),
    )?;
    let running = store.transition_attempt(
        &attempt.id,
        attempt.resource_version,
        v1::DevelopmentAttemptState::Running,
        None,
    )?;
    let succeeded = store.transition_attempt(
        &attempt.id,
        running.resource_version,
        v1::DevelopmentAttemptState::Succeeded,
        None,
    )?;
    let candidate = store.record_candidate(v1::Candidate {
        development_attempt_id: succeeded.id,
        repository_connection_id: repository.id,
        base_revision: REVISION.to_owned(),
        identity: Some(v1::CandidateIdentity {
            identity: Some(v1::candidate_identity::Identity::Git(v1::GitCandidate {
                commit_sha: SECOND_REVISION.to_owned(),
            })),
        }),
        state: v1::CandidateState::Available as i32,
        changed_paths: vec!["src/lib.rs".to_owned()],
        ..v1::Candidate::default()
    })?;
    let mut duplicate_candidate = candidate.clone();
    duplicate_candidate.id.clear();
    duplicate_candidate.created_at = None;
    duplicate_candidate.resource_version = 0;
    assert_eq!(
        store.record_candidate(duplicate_candidate)?.id,
        candidate.id
    );
    let request = v1::PublishCandidateRequest {
        idempotency_key: "publish".to_owned(),
        candidate_id: candidate.id,
        title: "feat: bounded change".to_owned(),
        body: "Body".to_owned(),
        base_branch: "main".to_owned(),
        draft: true,
    };
    let change_request = store.publish_candidate("publish-scope", &request)?;
    let action = store
        .durable_action("PublishCandidate", "publish-scope")?
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(action.state, DurableActionState::Pending);
    assert_eq!(
        v1::PublishCandidateRequest::decode(action.request_payload.as_slice())
            .map_err(|_| CleanStoreError::CorruptState)?,
        request
    );
    let provider = v1::ChangeRequestProvider {
        provider: Some(v1::change_request_provider::Provider::Github(
            v1::GithubChangeRequest {
                repository_id: "123".to_owned(),
                pull_request_number: 7,
                html_url: "https://github.com/ayni-oss/sandbox/pull/7".to_owned(),
            },
        )),
    };
    let failure = v1::ResourceFailure {
        reason: v1::ErrorReason::ProviderUnavailable as i32,
        message: "provider response was uncertain".to_owned(),
        remediation: None,
        observed_at: None,
    };
    let failed = store.reconcile_change_request(
        "publish-scope",
        change_request.resource_version,
        provider.clone(),
        v1::ChangeRequestState::Failed,
        SECOND_REVISION,
        Some(failure.clone()),
    )?;
    let duplicate = store.reconcile_change_request(
        "publish-scope",
        change_request.resource_version,
        provider,
        v1::ChangeRequestState::Failed,
        SECOND_REVISION,
        Some(failure),
    )?;
    assert_eq!(duplicate.resource_version, failed.resource_version);
    let pending =
        store.retry_change_request_publication("publish-scope", failed.resource_version)?;
    assert_eq!(pending.state, v1::ChangeRequestState::Publishing as i32);
    assert_eq!(
        store
            .durable_action("PublishCandidate", "publish-scope")?
            .map(|value| value.state),
        Some(DurableActionState::Pending)
    );
    assert_eq!(
        store.development_task(&task.id)?.state,
        v1::DevelopmentTaskState::Succeeded as i32
    );
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
    assert!(!timestamp_at_or_before(
        &deadline,
        &Timestamp {
            seconds: 11,
            nanos: 99_999_999,
        }
    ));
    assert!(timestamp_at_or_before(&deadline, &deadline));
    Ok(())
}

#[test]
fn reconciliation_expires_task_and_attempt_deadlines() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (task, attempt) = store.create_development_task(
        "task-scope",
        &task_request(&repository.id, &profile.id, "task"),
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
        AutomationInstallationState as Automation, CandidateState as Candidate,
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
                    Profile::Ready | Profile::Updating | Profile::Degraded | Profile::Failed
                ),
                Profile::Ready => matches!(next, Profile::Updating | Profile::Degraded),
                Profile::Updating => {
                    matches!(next, Profile::Ready | Profile::Degraded | Profile::Failed)
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

    let automation_states = [
        Automation::SetupRequired,
        Automation::SetupPrOpen,
        Automation::Installed,
        Automation::Updating,
        Automation::Degraded,
        Automation::Failed,
    ];
    for current in automation_states {
        for next in automation_states {
            let expected = match current {
                Automation::SetupRequired => matches!(
                    next,
                    Automation::SetupPrOpen
                        | Automation::Installed
                        | Automation::Degraded
                        | Automation::Failed
                ),
                Automation::SetupPrOpen => matches!(
                    next,
                    Automation::SetupRequired
                        | Automation::Installed
                        | Automation::Updating
                        | Automation::Degraded
                        | Automation::Failed
                ),
                Automation::Installed => {
                    matches!(next, Automation::Updating | Automation::Degraded)
                }
                Automation::Updating => matches!(
                    next,
                    Automation::Installed | Automation::Degraded | Automation::Failed
                ),
                Automation::Degraded => matches!(
                    next,
                    Automation::SetupRequired
                        | Automation::Updating
                        | Automation::Installed
                        | Automation::Failed
                ),
                Automation::Failed | Automation::Unspecified => false,
            };
            assert_eq!(valid_automation_transition(current, next), expected);
        }
    }
    assert!(valid_candidate_transition(
        Candidate::Available,
        Candidate::Rejected
    ));
    assert!(!valid_candidate_transition(
        Candidate::Rejected,
        Candidate::Available
    ));
    assert!(!valid_candidate_transition(
        Candidate::Available,
        Candidate::Available
    ));
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
            id: format!("repository:{version}"),
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
fn saved_work_publication_resumes_once_for_success_failure_and_deadline()
-> Result<(), Box<dyn std::error::Error>> {
    for state in [
        v1::DevelopmentAttemptState::Succeeded,
        v1::DevelopmentAttemptState::Failed,
        v1::DevelopmentAttemptState::DeadlineExceeded,
        v1::DevelopmentAttemptState::WaitingForInput,
    ] {
        retained_candidate_publishes_after_restart(state)?;
    }
    Ok(())
}

fn retained_candidate_publishes_after_restart(
    state: v1::DevelopmentAttemptState,
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("publication.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let mut request = task_request(&repository.id, &profile.id, "auto-task");
    request.request.as_mut().ok_or("request")?.publish_draft = false; // Legacy opt-out cannot discard new task work.
    let (task, attempt) = store.create_development_task("auto-task", &request)?;
    let frozen = task.snapshot.as_ref().ok_or("snapshot")?;
    assert!(frozen.publish_draft);
    assert_eq!(frozen.publication_base_branch, "main");
    let running = store.transition_attempt(
        &attempt.id,
        attempt.resource_version,
        v1::DevelopmentAttemptState::Running,
        None,
    )?;
    store.transition_attempt(&attempt.id, running.resource_version, state, None)?;
    let candidate = store.record_candidate(v1::Candidate {
        development_attempt_id: attempt.id,
        repository_connection_id: repository.id,
        base_revision: REVISION.into(),
        identity: Some(v1::CandidateIdentity {
            identity: Some(v1::candidate_identity::Identity::Git(v1::GitCandidate {
                commit_sha: SECOND_REVISION.into(),
            })),
        }),
        state: v1::CandidateState::Available as i32,
        changed_paths: vec!["src/lib.rs".into()],
        ..Default::default()
    })?;
    if state == v1::DevelopmentAttemptState::Failed {
        store.retry_development_task(
            "retry-with-checkpoint",
            &v1::RetryDevelopmentTaskRequest {
                idempotency_key: "retry-with-checkpoint".into(),
                development_task_id: task.id.clone(),
            },
        )?;
    }
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    store.schedule_requested_publications()?;
    store.schedule_requested_publications()?;
    let actions = store.pending_external_actions("PublishCandidate")?;
    assert_eq!(actions.len(), 1);
    let publication = v1::PublishCandidateRequest::decode(actions[0].request_payload.as_slice())?;
    assert_eq!(publication.candidate_id, candidate.id);
    assert_eq!(publication.base_branch, "main");
    assert!(publication.draft);
    assert_eq!(store.development_task(&task.id)?.snapshot, task.snapshot);
    Ok(())
}

#[test]
fn setup_observation_recovers_from_provider_failure() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let repository = store
        .create_repository_connection("setup-repo", &repository_request("setup-repo", "123"))?;
    let snapshot = store.observe_repository_snapshot(&repository.id, REVISION)?;
    let (profile, _) = store.create_runtime_profile(
        "setup-profile",
        &profile_request("setup-profile", &repository.id, &snapshot.id),
    )?;
    let installation = store.install_repository_automation(
        "setup-recovery",
        &v1::InstallRepositoryAutomationRequest {
            idempotency_key: "setup-recovery".into(),
            runtime_profile_id: profile.id,
        },
    )?;
    store.observe_setup(
        &installation.id,
        Some(8),
        v1::AutomationInstallationState::Degraded,
        Some("github_unavailable"),
    )?;
    store.observe_setup(
        &installation.id,
        Some(8),
        v1::AutomationInstallationState::SetupPrOpen,
        None,
    )?;
    store.observe_setup(
        &installation.id,
        Some(8),
        v1::AutomationInstallationState::Installed,
        None,
    )?;
    let observed = store.automation_installation(&installation.id)?;
    assert_eq!(
        observed.state,
        v1::AutomationInstallationState::Installed as i32
    );
    assert!(observed.failure.is_none());
    Ok(())
}

#[test]
fn matching_workflows_complete_setup_without_a_pull_request() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let repository = store
        .create_repository_connection("reuse-repo", &repository_request("reuse-repo", "123"))?;
    let snapshot = store.observe_repository_snapshot(&repository.id, REVISION)?;
    let (profile, _) = store.create_runtime_profile(
        "reuse-profile",
        &profile_request("reuse-profile", &repository.id, &snapshot.id),
    )?;
    let installation = store.install_repository_automation(
        "reuse-setup",
        &v1::InstallRepositoryAutomationRequest {
            idempotency_key: "reuse-setup".into(),
            runtime_profile_id: profile.id.clone(),
        },
    )?;
    for _ in 0..2 {
        store.observe_setup(
            &installation.id,
            None,
            v1::AutomationInstallationState::Installed,
            None,
        )?;
        store.complete_external_action("InstallRepositoryAutomation", "reuse-setup")?;
    }
    let installed = store.automation_installation(&installation.id)?;
    assert_eq!(
        installed.state,
        v1::AutomationInstallationState::Installed as i32
    );
    let Some(v1::automation_installation::Provider::Github(github)) = installed.provider else {
        return Err(CleanStoreError::CorruptState);
    };
    assert!(github.setup_pull_request_number.is_none());
    assert!(
        store
            .pending_external_actions("InstallRepositoryAutomation")?
            .is_empty()
    );
    assert_eq!(
        store.runtime_profile(&profile.id)?.state,
        v1::RuntimeProfileState::AwaitingEnvironment as i32
    );
    Ok(())
}
