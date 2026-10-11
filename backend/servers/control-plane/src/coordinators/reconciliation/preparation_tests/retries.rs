use super::*;

#[tokio::test]
async fn terminal_execution_is_counted_once_across_restart_and_cleanup_errors() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let name = fixture.pending()?.name.clone();
    for _ in 0..4 {
        assert_eq!(
            fixture
                .reconciler
                .fail_execution(
                    &fixture.profile,
                    &mut fixture.cursor,
                    &name,
                    "runtime_verification_failed"
                )
                .await,
            Err("runtime_verification_failed")
        );
        fixture
            .reconciler
            .record_failure(&fixture.profile, "kubernetes_unavailable")
            .await?;
        let id = fixture.profile.clone();
        fixture.reconciler.state =
            AppState::open(fixture.directory.path().join("state.sqlite"), 3)?;
        fixture.cursor = fixture
            .reconciler
            .state
            .database
            .execute(move |store| store.runtime_cursor(&id))
            .await?;
        fixture
            .reconciler
            .record_failure(&fixture.profile, "runtime_cleanup_failed")
            .await?;
    }
    assert_eq!(fixture.cursor.failures, 1);
    assert_eq!(fixture.cursor.accounting.executions.len(), 1);
    let history = fixture.cursor.accounting.executions.clone();
    fixture.cursor.retry();
    assert_eq!(fixture.cursor.failures, 0);
    assert_eq!(fixture.cursor.accounting.executions, history);
    Ok(())
}

#[tokio::test]
async fn three_distinct_failed_executions_exhaust_without_a_retry_timestamp() -> TestResult {
    let mut fixture = Fixture::new().await?;
    for attempt in 0..3 {
        let name = format!("execution-{attempt}");
        assert!(
            fixture
                .reconciler
                .fail_execution(
                    &fixture.profile,
                    &mut fixture.cursor,
                    &name,
                    "runtime_build_failed"
                )
                .await
                .is_err()
        );
    }
    assert_eq!(fixture.cursor.failures, 3);
    assert_eq!(fixture.cursor.accounting.executions.len(), 3);
    assert!(fixture.cursor.retry_exhausted());
    assert!(
        !fixture
            .cursor
            .metadata()
            .iter()
            .any(|entry| entry.key == "next-retry")
    );
    fixture.step().await?;
    assert!(fixture.pending()?.failed);
    Ok(())
}

#[tokio::test]
async fn reporting_and_observation_errors_preserve_execution_and_budget() -> TestResult {
    let fixture = Fixture::new().await?;
    let frozen = serde_json::to_value(&fixture.cursor.pending)?;
    for reason in [
        "kubernetes_unavailable",
        "runtime_observation_timeout",
        "runtime_report_missing",
        "runtime_reporting_failed",
        "runtime_cleanup_failed",
    ] {
        fixture
            .reconciler
            .record_failure(&fixture.profile, reason)
            .await?;
    }
    let id = fixture.profile.clone();
    let stored = fixture
        .reconciler
        .state
        .database
        .execute(move |store| store.runtime_cursor(&id))
        .await?;
    assert_eq!(stored.failures, 0);
    assert!(stored.accounting.executions.is_empty());
    assert_eq!(serde_json::to_value(&stored.pending)?, frozen);
    assert_eq!(
        stored
            .accounting
            .operational
            .get(&ayni_orchestrator_application::runtime_preparation::FailureClass::Observation),
        Some(&2)
    );
    assert_eq!(
        stored
            .accounting
            .operational
            .get(&ayni_orchestrator_application::runtime_preparation::FailureClass::Reporting),
        Some(&2)
    );
    assert_eq!(
        stored
            .accounting
            .operational
            .get(&ayni_orchestrator_application::runtime_preparation::FailureClass::Cleanup),
        Some(&1)
    );
    Ok(())
}

#[tokio::test]
async fn failed_build_waits_for_cleanup_before_replacement() -> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture.discovery(ImageDiscovery::Missing);
    fixture.step().await?;
    fixture.step().await?;
    let build = fixture.pending()?.build_name.clone().ok_or("build")?;
    fixture.complete(&build, ExecutionState::Failed);
    assert_eq!(fixture.step().await, Err("runtime_build_failed"));
    assert_eq!(fixture.cursor.failures, 1);
    fixture
        .reconciler
        .kubernetes
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .cleanup_pending = true;
    assert_eq!(fixture.step().await, Err("runtime_cleanup_pending"));
    assert_eq!(fixture.cursor.failures, 1);
    assert!(fixture.pending()?.failed);
    let id = fixture.profile.clone();
    fixture.reconciler.state = AppState::open(fixture.directory.path().join("state.sqlite"), 3)?;
    fixture.cursor = fixture
        .reconciler
        .state
        .database
        .execute(move |store| store.runtime_cursor(&id))
        .await?;
    fixture
        .reconciler
        .kubernetes
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .cleanup_pending = false;
    fixture.step().await?;
    assert!(fixture.cursor.pending.is_none());
    assert_eq!(fixture.cursor.failures, 1);
    assert!(fixture.cursor.accounting.executions.contains_key(&build));
    Ok(())
}

#[tokio::test]
async fn explicit_retry_admission_survives_replay_without_replenishing_used_budget() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let id = fixture.profile.clone();
    let request = v1::ReconcileRuntimeProfileRequest {
        idempotency_key: "explicit-retry".into(),
        runtime_profile_id: id.clone(),
        force_rebuild: false,
    };
    fixture
        .cursor
        .accounting
        .executions
        .insert("old-execution".into(), "runtime_build_failed".into());
    fixture.cursor.failures = 3;
    fixture.reconciler.save(&id, &fixture.cursor).await?;
    fixture
        .reconciler
        .state
        .database
        .execute(move |store| store.reconcile_runtime_profile("explicit-retry", &request))
        .await?;
    let selection = fixture
        .reconciler
        .select_profile(&fixture.profile)
        .await?
        .ok_or("selection")?;
    assert_eq!(selection.cursor.failures, 0);
    assert_eq!(selection.cursor.accounting.executions.len(), 1);
    fixture.cursor = selection.cursor;
    fixture.cursor.failures = 1;
    fixture
        .reconciler
        .save(&fixture.profile, &fixture.cursor)
        .await?;
    // Simulate failure to acknowledge after the admission itself was committed.
    rusqlite::Connection::open(fixture.directory.path().join("state.sqlite"))?.execute(
        "UPDATE durable_actions SET state='pending' WHERE idempotency_key='explicit-retry'",
        [],
    )?;
    let selection = fixture
        .reconciler
        .select_profile(&fixture.profile)
        .await?
        .ok_or("selection")?;
    assert_eq!(selection.cursor.failures, 1);
    assert_eq!(selection.cursor.accounting.admissions.len(), 1);
    Ok(())
}

/// Run inside the local chart's control-plane container against a scoped failed Job.
/// The Job must have the `verify` container and emit `runtime_lock_mismatch`.
#[tokio::test]
#[ignore = "requires an in-cluster client and AYNI_RETRY_FIXTURE Job in Docker Desktop"]
async fn failed_kubernetes_job_retains_one_outcome_across_restart() -> TestResult {
    let fixture = Fixture::new().await?;
    let name = std::env::var("AYNI_RETRY_FIXTURE")?;
    let mut configuration = fixture.reconciler.configuration.clone();
    configuration.namespace = "ayni-orchestrator-dev".into();
    let reconciler = RuntimeReconciler {
        state: fixture.reconciler.state.clone(),
        kubernetes: RuntimeKubernetes::new(configuration.execution_configuration())?,
        configuration,
        source: fixture.reconciler.source.clone(),
        pass: Mutex::new(()),
        coding_pass: Mutex::new(()),
        credential_refresh: Mutex::new(BTreeMap::new()),
    };
    for _ in 0..2 {
        assert!(matches!(
            reconciler.kubernetes.observe(&name).await?,
            Observation::Failed("runtime_lock_mismatch")
        ));
    }
    let mut cursor = fixture.cursor;
    let pending = cursor.pending.as_mut().ok_or("pending")?;
    pending.name.clone_from(&name);
    pending.phase = ImagePreparationPhase::Verifying;
    pending.resolved_digest = Some(format!("sha256:{}", "d".repeat(64)));
    let observation = ProfileObservation {
        id: &fixture.profile,
        runtime: &fixture.runtime,
        time: 2,
        github: v1::GithubRepository::default(),
        head: "a".repeat(40),
        token: String::new(),
        source_digest: "frozen".into(),
        default_branch: "main".into(),
        selected_revision: None,
        registry: v1::RuntimeRegistrySelection::default(),
        certificate: None,
    };
    assert_eq!(
        reconciler.observe_pending(&observation, &mut cursor).await,
        Err("runtime_lock_mismatch")
    );
    let id = fixture.profile.clone();
    let state = AppState::open(fixture.directory.path().join("state.sqlite"), 3)?;
    cursor = state
        .database
        .execute(move |store| store.runtime_cursor(&id))
        .await?;
    assert_eq!(cursor.failures, 1);
    assert_eq!(cursor.accounting.executions.len(), 1);
    assert!(
        cursor
            .pending
            .as_ref()
            .is_some_and(|pending| pending.failed)
    );
    // A restarted reconciler resumes cleanup, never observes this as a new failure.
    match reconciler.observe_pending(&observation, &mut cursor).await {
        Ok(()) | Err("runtime_cleanup_pending") => {}
        Err(reason) => return Err(reason.into()),
    }
    assert_eq!(cursor.failures, 1);
    assert_eq!(
        cursor.accounting.executions.get(&name).map(String::as_str),
        Some("runtime_lock_mismatch")
    );
    Ok(())
}

#[tokio::test]
async fn only_changed_frozen_inputs_replenish_a_preparation_budget() -> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture.cursor.pending = None;
    let identity = crate::profiles::ayni::environment::LockIdentity {
        fingerprint: "c".repeat(64),
        ayni_version: "0.15.0".into(),
    };
    let registry = v1::RuntimeRegistrySelection::default();
    assert!(
        fixture
            .reconciler
            .freeze_preparation(
                &fixture.profile,
                &fixture.runtime,
                "source",
                b"policy",
                b"lock",
                &identity,
                &registry,
                None,
                &mut fixture.cursor
            )
            .await?
    );
    fixture.cursor.failures = 3;
    fixture
        .cursor
        .accounting
        .executions
        .insert("failed-execution".into(), "runtime_build_failed".into());
    fixture.cursor.accounting.blocked = true;
    assert!(
        fixture
            .reconciler
            .freeze_preparation(
                &fixture.profile,
                &fixture.runtime,
                "source",
                b"policy",
                b"lock",
                &identity,
                &registry,
                None,
                &mut fixture.cursor
            )
            .await?
    );
    assert_eq!(fixture.cursor.failures, 3);
    assert!(fixture.cursor.retry_exhausted());
    assert!(
        fixture
            .reconciler
            .freeze_preparation(
                &fixture.profile,
                &fixture.runtime,
                "source",
                b"changed-policy",
                b"lock",
                &identity,
                &registry,
                None,
                &mut fixture.cursor
            )
            .await?
    );
    assert_eq!(fixture.cursor.failures, 0);
    assert!(!fixture.cursor.retry_exhausted());
    assert_eq!(fixture.cursor.accounting.executions.len(), 1);
    Ok(())
}

#[tokio::test]
async fn explicit_reporting_retry_reexecutes_only_unrecoverable_terminal_evidence() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let frozen = fixture.pending()?.clone();
    for (reason, reexecute) in [
        ("runtime_report_missing", true),
        ("runtime_report_unavailable", false),
    ] {
        fixture.cursor.pending = Some(frozen.clone());
        fixture
            .cursor
            .status
            .insert("reason-code".into(), reason.into());
        fixture
            .reconciler
            .save(&fixture.profile, &fixture.cursor)
            .await?;
        let request = v1::ReconcileRuntimeProfileRequest {
            idempotency_key: reason.into(),
            runtime_profile_id: fixture.profile.clone(),
            force_rebuild: false,
        };
        fixture
            .reconciler
            .state
            .database
            .execute(move |store| store.reconcile_runtime_profile(reason, &request))
            .await?;
        let selection = fixture
            .reconciler
            .select_profile(&fixture.profile)
            .await?
            .ok_or("selection")?;
        assert_eq!(
            selection
                .cursor
                .pending
                .as_ref()
                .ok_or("pending")?
                .source_digest
                .is_empty(),
            reexecute
        );
        assert_eq!(selection.cursor.failures, 0);
        assert!(selection.cursor.accounting.executions.is_empty());
        assert!(!selection.cursor.force_rebuild);
        fixture.cursor = selection.cursor;
    }
    Ok(())
}

#[tokio::test]
async fn terminal_job_cleanup_progresses_before_unavailable_source_observation() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let name = fixture.pending()?.name.clone();
    fixture.cursor.pending.as_mut().ok_or("pending")?.failed = true;
    fixture.cursor.failures = 3;
    fixture.complete(&name, ExecutionState::Failed);
    fixture
        .reconciler
        .save(&fixture.profile, &fixture.cursor)
        .await?;
    assert!(fixture.reconciler.profile(&fixture.profile).await.is_err());
    assert!(
        fixture
            .reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .states
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn failed_verification_observations_without_trustworthy_evidence_preserve_budget()
-> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture.cursor.pending.as_mut().ok_or("pending")?.phase = ImagePreparationPhase::Verifying;
    let frozen = serde_json::to_value(&fixture.cursor.pending)?;
    for reason in [
        "runtime_report_missing",
        "runtime_reporting_failed",
        "runtime_pod_identity_invalid",
    ] {
        fixture
            .reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .verification_failure = Some(reason);
        for _ in 0..3 {
            assert_eq!(fixture.step().await, Err(reason));
            assert_eq!(fixture.cursor.failures, 0);
            assert_eq!(serde_json::to_value(&fixture.cursor.pending)?, frozen);
        }
    }
    assert!(fixture.cursor.accounting.executions.is_empty());
    Ok(())
}
