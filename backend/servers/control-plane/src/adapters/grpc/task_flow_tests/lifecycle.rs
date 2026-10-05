use super::*;

#[tokio::test]
async fn task_admission_requires_ready_matching_source_and_freezes_inputs() -> TestResult {
    let directory = tempfile::tempdir()?;
    let (server, channel) = Server::start(&directory.path().join("state.sqlite")).await?;
    let profile = create_profile(&server, channel.clone()).await?;
    let mut client = TaskClient::new(channel.clone());
    let request = task_request(&profile, "create-task");
    let unready = client.create_development_task(request.clone()).await;
    assert_error(
        unready,
        Code::FailedPrecondition,
        v1::ErrorReason::ProfileNotReady,
    )?;
    let artifact =
        observe_verified_runtime(&server, &profile, &format!("sha256:{}", "a".repeat(64)))?;
    let ready = RuntimeProfileServiceClient::new(channel)
        .get_runtime_profile(v1::GetRuntimeProfileRequest {
            runtime_profile_id: profile.profile.id.clone(),
        })
        .await?
        .into_inner()
        .runtime_profile
        .ok_or("ready profile missing")?;
    assert_eq!(ready.state, v1::RuntimeProfileState::Ready as i32);
    let created = client
        .create_development_task(request.clone())
        .await?
        .into_inner();
    let task = created.development_task.as_ref().ok_or("task missing")?;
    let attempt = created.initial_attempt.as_ref().ok_or("attempt missing")?;
    assert_frozen_inputs(&profile, &artifact, &request, task, attempt)?;
    assert_eq!(
        client
            .create_development_task(request.clone())
            .await?
            .into_inner(),
        created
    );
    assert_eq!(get_task(&mut client, &task.id).await?, *task);
    let listed = client
        .list_development_tasks(v1::ListDevelopmentTasksRequest {
            repository_connection_id: Some(profile.repository.id.clone()),
            runtime_profile_id: Some(profile.profile.id.clone()),
            ..Default::default()
        })
        .await?
        .into_inner();
    assert_eq!(listed.development_tasks, vec![task.clone()]);
    let mut conflict = request.clone();
    conflict.request.as_mut().ok_or("request missing")?.request = "Different work".into();
    assert_error(
        client.create_development_task(conflict).await,
        Code::AlreadyExists,
        v1::ErrorReason::IdempotencyKeyReused,
    )?;
    with_store(&server.state, |store| {
        store.observe_repository_snapshot(&profile.repository.id, &"b".repeat(40))
    })?;
    let mut stale = request;
    stale.idempotency_key = "stale-source".into();
    stale
        .request
        .as_mut()
        .ok_or("request missing")?
        .repository_revision = "b".repeat(40);
    assert_error(
        client.create_development_task(stale).await,
        Code::FailedPrecondition,
        v1::ErrorReason::ProfileNotReady,
    )?;
    server.stop().await
}

#[tokio::test]
async fn cancellation_and_resumable_watch_survive_server_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("state.sqlite");
    let (server, channel) = Server::start(&path).await?;
    let profile = create_profile(&server, channel.clone()).await?;
    observe_verified_runtime(&server, &profile, &format!("sha256:{}", "a".repeat(64)))?;
    let mut client = TaskClient::new(channel);
    let created = client
        .create_development_task(task_request(&profile, "task"))
        .await?
        .into_inner();
    let task = created.development_task.ok_or("task missing")?;
    let attempt = created.initial_attempt.ok_or("attempt missing")?;
    let mut stream = client
        .watch_development_task(v1::WatchDevelopmentTaskRequest {
            development_task_id: task.id.clone(),
            after_cursor: None,
        })
        .await?
        .into_inner();
    let first = next_update(&mut stream).await?;
    let second = next_update(&mut stream).await?;
    let cursor = first.cursor.max(second.cursor);
    assert!(
        [&first.update, &second.update]
            .iter()
            .any(|value| matches!(value, Some(Update::DevelopmentTask(value)) if value == &task))
    );
    assert!(
        [&first.update, &second.update]
            .iter()
            .any(|value| matches!(value, Some(Update::Attempt(value)) if value == &attempt))
    );
    drop(stream);
    let cancel = v1::CancelDevelopmentTaskRequest {
        idempotency_key: "cancel".into(),
        development_task_id: task.id.clone(),
    };
    let cancelled = client
        .cancel_development_task(cancel.clone())
        .await?
        .into_inner()
        .development_task
        .ok_or("cancelled task missing")?;
    assert_eq!(cancelled.state, v1::DevelopmentTaskState::Cancelled as i32);
    server.stop().await?;
    let (server, channel) = Server::start(&path).await?;
    let mut client = TaskClient::new(channel);
    assert_eq!(get_task(&mut client, &task.id).await?, cancelled);
    assert_eq!(
        client
            .cancel_development_task(cancel)
            .await?
            .into_inner()
            .development_task,
        Some(cancelled.clone())
    );
    let mut stream = client
        .watch_development_task(v1::WatchDevelopmentTaskRequest {
            development_task_id: task.id.clone(),
            after_cursor: Some(cursor),
        })
        .await?
        .into_inner();
    let first = next_update(&mut stream).await?;
    let second = next_update(&mut stream).await?;
    assert!(first.cursor > cursor && second.cursor > first.cursor);
    assert!(
        [&first.update, &second.update].iter().any(
            |value| matches!(value, Some(Update::DevelopmentTask(value)) if value == &cancelled)
        )
    );
    assert!([&first.update, &second.update].iter().any(|value| matches!(value, Some(Update::Attempt(value)) if value.id == attempt.id && value.state == v1::DevelopmentAttemptState::Cancelled as i32)));
    assert_error(
        client
            .retry_development_task(v1::RetryDevelopmentTaskRequest {
                idempotency_key: "retry-cancelled".into(),
                development_task_id: task.id,
            })
            .await,
        Code::FailedPrecondition,
        v1::ErrorReason::TaskNotActionable,
    )?;
    drop(stream);
    server.stop().await
}

fn observe_failure_and_new_profile(
    server: &Server,
    profile: &RuntimeFixture,
    attempt: &v1::DevelopmentAttempt,
) -> TestResult {
    // These are observations the worker collector and repository reconciler
    // would deliver. All user mutations still go through the gRPC listener.
    with_store(&server.state, |store| {
        let running = store.transition_attempt(
            &attempt.id,
            attempt.resource_version,
            v1::DevelopmentAttemptState::Running,
            None,
        )?;
        store.transition_attempt(
            &attempt.id,
            running.resource_version,
            v1::DevelopmentAttemptState::Failed,
            None,
        )?;
        let source = store.observe_repository_snapshot(&profile.repository.id, &"b".repeat(40))?;
        store.record_runtime_profile_revision(
            &profile.profile.id,
            &source.id,
            profile
                .revision
                .configuration
                .clone()
                .ok_or(crate::adapters::sqlite::CleanStoreError::CorruptState)?,
        )
    })?;
    Ok(())
}

#[tokio::test]
async fn retry_replays_once_and_keeps_frozen_inputs_after_profile_change_and_restart() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("state.sqlite");
    let (server, channel) = Server::start(&path).await?;
    let profile = create_profile(&server, channel.clone()).await?;
    observe_verified_runtime(&server, &profile, &format!("sha256:{}", "a".repeat(64)))?;
    let mut client = TaskClient::new(channel);
    let create = task_request(&profile, "task");
    let created = client
        .create_development_task(create.clone())
        .await?
        .into_inner();
    let task = created.development_task.ok_or("task missing")?;
    let initial = created.initial_attempt.ok_or("attempt missing")?;
    observe_failure_and_new_profile(&server, &profile, &initial)?;
    let current_profile = with_store(&server.state, |store| {
        store.runtime_profile(&profile.profile.id)
    })?;
    assert_ne!(
        current_profile.active_revision_id.as_deref(),
        Some(profile.revision.id.as_str())
    );
    assert_eq!(
        current_profile.state,
        v1::RuntimeProfileState::Updating as i32
    );
    assert_eq!(
        get_task(&mut client, &task.id).await?.state,
        v1::DevelopmentTaskState::Failed as i32
    );
    server.stop().await?;
    let (server, channel) = Server::start(&path).await?;
    let mut client = TaskClient::new(channel);
    let retry = v1::RetryDevelopmentTaskRequest {
        idempotency_key: "retry".into(),
        development_task_id: task.id.clone(),
    };
    let attempt = client
        .retry_development_task(retry.clone())
        .await?
        .into_inner()
        .attempt
        .ok_or("retried attempt missing")?;
    assert_eq!(attempt.attempt_number, 2);
    assert_ne!(attempt.id, initial.id);
    assert_eq!(attempt.snapshot, task.snapshot);
    server.stop().await?;
    let (server, channel) = Server::start(&path).await?;
    let mut client = TaskClient::new(channel);
    assert_eq!(
        client
            .retry_development_task(retry)
            .await?
            .into_inner()
            .attempt,
        Some(attempt.clone())
    );
    let replay = client.create_development_task(create).await?.into_inner();
    assert_eq!(
        replay
            .development_task
            .as_ref()
            .ok_or("replayed task missing")?
            .id,
        task.id
    );
    assert_eq!(
        replay
            .initial_attempt
            .as_ref()
            .ok_or("replayed attempt missing")?
            .id,
        initial.id
    );
    let attempts = client
        .list_development_attempts(v1::ListDevelopmentAttemptsRequest {
            development_task_id: task.id.clone(),
            page: None,
        })
        .await?
        .into_inner()
        .attempts;
    assert_eq!(attempts.len(), 2);
    assert!(
        attempts.iter().any(|value| value.id == initial.id
            && value.state == v1::DevelopmentAttemptState::Failed as i32)
    );
    assert!(attempts.contains(&attempt));
    let current = get_task(&mut client, &task.id).await?;
    assert_eq!(current.attempt_count, 2);
    assert_eq!(current.active_attempt_id, Some(attempt.id));
    assert_eq!(current.snapshot, task.snapshot);
    server.stop().await
}
