//! Serial task ownership, fencing, exact command receipts and retained result acceptance.
use ayni_orchestrator_events::task_execution::{
    SupervisorRequest as S, TaskOutcome, WorkerRequest as W,
};
use ayni_orchestrator_events::task_quality::{QualityOutcome, snapshot_digest};
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, connect,
    execution::ExecutionInputs,
    migrate_for_runtime,
    plans::{PlanContent, PlanScope, PlanTaskSpecification},
};
use std::error::Error;
use uuid::Uuid;
#[path = "support/task_recovery.rs"]
mod recovery;
type Result = std::result::Result<(), Box<dyn Error + Send + Sync>>;
fn key() -> String {
    Uuid::now_v7().to_string()
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and distinct runtime role"]
#[allow(
    clippy::too_many_lines,
    reason = "one ownership scenario keeps fencing and serial-slot ordering explicit"
)]
async fn serial_dispatch_fences_cancellation_and_recovers_exact_retained_results() -> Result {
    let (admin, runtime, store) = store_fixture().await?;
    let workspace = store.create_workspace(&key(), "Task runs").await?.resource;
    let project = store
        .create_project(&key(), &workspace.id, "Task runs", "Serial coding")
        .await?
        .resource;
    let conversation = store
        .create_conversation(&key(), &workspace.id, &project.id, "Origin")
        .await?
        .resource;
    let other = store
        .create_conversation(&key(), &workspace.id, &project.id, "Other")
        .await?
        .resource;
    let scope = PlanScope {
        workspace: &workspace.id,
        project: &project.id,
        conversation: &conversation.id,
    };
    let wrong = PlanScope {
        conversation: &other.id,
        ..scope
    };
    let content = PlanContent {
        title: "Execute tasks".into(),
        objective: "Retain changes".into(),
        scope: "One active worker".into(),
        tasks: (0..8)
            .map(|n| PlanTaskSpecification {
                key: format!("task{n}"),
                objective: "One task".into(),
                scope: "Bounded".into(),
                acceptance_criteria: vec!["Frozen result".into()],
                repository: "https://example.invalid/source.git".into(),
                source_ref: "refs/heads/main".into(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let version = store
        .create_plan(scope, &key(), "", &content)
        .await?
        .version;
    let approved = store
        .approve_plan(
            scope,
            &key(),
            &version.plan_id,
            &version.id,
            &version.content_digest,
        )
        .await?;
    let inputs = ExecutionInputs {
        repository: "https://example.invalid/source.git".into(),
        source_ref: "refs/heads/main".into(),
        source_commit: "a".repeat(40),
        lock_digest: format!("sha256:{}", "b".repeat(64)),
        runtime_image: format!("example.invalid/runtime@sha256:{}", "c".repeat(64)),
        max_seconds: 600,
        max_tokens: 10000,
    };
    let mut preparations = Vec::new();
    for task in &approved.tasks {
        preparations.push(
            store
                .prepare_task_execution(scope, &key(), &task.id, &inputs)
                .await?,
        );
    }
    let first = &preparations[0];
    let second = &preparations[1];
    let queued_cancel = &preparations[2];
    let deadline = &preparations[3];
    let start_key = key();
    assert!(matches!(
        store
            .start_task_execution(
                wrong,
                &first.task_id,
                &first.attempt_id,
                &key(),
                b"fixture-pack"
            )
            .await,
        Err(StoreError::NotFound)
    ));
    let receipt = store
        .start_task_execution(
            scope,
            &first.task_id,
            &first.attempt_id,
            &start_key,
            b"fixture-pack",
        )
        .await?;
    assert_eq!(
        store
            .start_task_execution(
                scope,
                &first.task_id,
                &first.attempt_id,
                &start_key,
                b"fixture-pack"
            )
            .await?,
        receipt
    );
    assert!(matches!(
        store
            .start_task_execution(
                scope,
                &first.task_id,
                &first.attempt_id,
                &start_key,
                b"changed-pack"
            )
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    for p in [second, queued_cancel] {
        store
            .start_task_execution(scope, &p.task_id, &p.attempt_id, &key(), b"fixture-pack")
            .await?;
    }
    cancel_queued_task(&store, scope, queued_cancel).await?;
    let (one_claim, other_claim) = tokio::join!(
        store.task_supervisor(&S::Poll),
        store.task_supervisor(&S::Poll)
    );
    let owned = one_claim?.ok_or("claim missing")?;
    assert_eq!(Some(owned.clone()), other_claim?);
    assert_eq!(owned.command.attempt_id, first.attempt_id);
    assert_eq!(
        store
            .get_task_run(scope, &second.task_id, &second.attempt_id)
            .await?
            .state,
        "queued"
    );
    // Kubernetes Pod identity is independent of our UUIDv7 resource IDs.
    let uid = "3c828c6e-53ed-4e39-bd68-c1198e69467c".to_owned();
    let observe = S::Observe {
        attempt_id: first.attempt_id.clone(),
        generation: 1,
        pod_uid: uid.clone(),
    };
    store.task_supervisor(&observe).await?;
    let source = store
        .task_worker(&first.attempt_id, 1, &uid, &W::Input)
        .await?;
    assert_eq!(source.source_pack, b"fixture-pack");
    assert_eq!(
        source
            .command
            .as_ref()
            .map(|queued_cancel| &queued_cancel.source_commit),
        Some(&inputs.source_commit)
    );
    let outcome = cancel_active_call(&store, scope, first, &owned.command, &uid).await?;
    let next = store
        .task_supervisor(&S::Poll)
        .await?
        .ok_or("next claim missing")?;
    assert_eq!(next.command.attempt_id, second.attempt_id);
    let uid = key();
    store
        .task_supervisor(&S::Observe {
            attempt_id: second.attempt_id.clone(),
            generation: 1,
            pod_uid: uid.clone(),
        })
        .await?;
    let report = W::Report {
        outcome: outcome.clone(),
    };
    assert!(
        store
            .task_worker(&second.attempt_id, 1, &uid, &report)
            .await?
            .authorized
    );
    assert!(
        store
            .task_worker(&second.attempt_id, 1, &uid, &report)
            .await?
            .authorized
    );
    let mut conflict = outcome.clone();
    conflict.patch = "different".into();
    assert!(
        store
            .task_worker(
                &second.attempt_id,
                1,
                &uid,
                &W::Report { outcome: conflict }
            )
            .await
            .is_err()
    );
    assert_eq!(
        store
            .task_supervisor(&S::Poll)
            .await?
            .ok_or("reporting")?
            .state,
        "reporting"
    );
    let command = store
        .get_task_run(scope, &second.task_id, &second.attempt_id)
        .await?
        .command;
    let quality = failed_quality(&command, &outcome);
    prove_quality_rollback(&store, second, &uid, &quality, &admin).await?;
    verify_quality_receipts(&store, second, &uid, &quality).await?;
    store
        .task_supervisor(&S::Stopped {
            attempt_id: second.attempt_id.clone(),
            generation: 1,
            pod_uid: uid.clone(),
        })
        .await?;
    let reopened = WorkflowStore::new(runtime.clone());
    let retained = reopened
        .get_task_run(scope, &second.task_id, &second.attempt_id)
        .await?;
    assert_eq!(retained.state, "completed");
    assert_eq!(retained.outcome, Some(outcome));
    assert_eq!(retained.quality, Some(quality.clone()));
    assert_eq!(
        quality.state, "failed",
        "quality failure must preserve completed coding"
    );
    verify_quality_receipts(&reopened, second, &uid, &quality).await?;
    assert!(
        reopened
            .task_worker(&second.attempt_id, 1, &uid, &report)
            .await?
            .authorized
    );
    assert_eq!(
        reopened
            .start_task_execution(
                scope,
                &first.task_id,
                &first.attempt_id,
                &start_key,
                b"fixture-pack"
            )
            .await?,
        receipt
    );
    assert!(reopened.task_supervisor(&S::Poll).await?.is_none());
    assert!(
        reopened
            .get_task_run(wrong, &second.task_id, &second.attempt_id)
            .await
            .is_err()
    );
    prove_rollback_deadline_and_immutable_inputs(&store, scope, deadline, first, &admin, &runtime)
        .await?;
    prove_retained_review(&store, scope, wrong, &preparations, &runtime).await
}
async fn prove_rollback_deadline_and_immutable_inputs(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    deadline: &ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
    first: &ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
    admin: &str,
    runtime: &str,
) -> Result {
    // Crash during command receipt insertion rolls back dispatch and its observation too.
    let (db, driver) = connect(admin).await?;
    db.batch_execute("CREATE FUNCTION fixture_reject_task_receipt() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture'; END $$; CREATE TRIGGER fixture_reject_task_receipt BEFORE INSERT ON workflow_task_run_receipts FOR EACH ROW EXECUTE FUNCTION fixture_reject_task_receipt()").await?;
    let result = store
        .start_task_execution(
            scope,
            &deadline.task_id,
            &deadline.attempt_id,
            &key(),
            b"fixture-pack",
        )
        .await;
    db.batch_execute("DROP TRIGGER fixture_reject_task_receipt ON workflow_task_run_receipts; DROP FUNCTION fixture_reject_task_receipt()").await?;
    assert!(matches!(result, Err(StoreError::Database(_))));
    assert!(matches!(
        store
            .get_task_run(scope, &deadline.task_id, &deadline.attempt_id)
            .await,
        Err(StoreError::NotFound)
    ));
    store
        .start_task_execution(
            scope,
            &deadline.task_id,
            &deadline.attempt_id,
            &key(),
            b"fixture-pack",
        )
        .await?;
    store.task_supervisor(&S::Poll).await?;
    db.execute("UPDATE workflow_task_runs SET deadline_at=clock_timestamp()-interval '1 second' WHERE attempt_id=$1",&[&deadline.attempt_id]).await?;
    assert_eq!(
        store
            .task_supervisor(&S::Poll)
            .await?
            .ok_or("expired")?
            .state,
        "cancelling"
    );
    store
        .task_supervisor(&S::Stopped {
            attempt_id: deadline.attempt_id.clone(),
            generation: 1,
            pod_uid: String::new(),
        })
        .await?;
    let (runtime, runtime_driver) = connect(runtime).await?;
    assert!(
        runtime
            .execute(
                "UPDATE workflow_task_runs SET command=command WHERE attempt_id=$1",
                &[&first.attempt_id]
            )
            .await
            .is_err()
    );
    assert!(
        runtime
            .execute(
                "DELETE FROM workflow_task_runs WHERE attempt_id=$1",
                &[&first.attempt_id]
            )
            .await
            .is_err()
    );
    assert!(
        runtime
            .execute(
                "UPDATE workflow_task_quality SET outcome=outcome WHERE attempt_id=$1",
                &[&first.attempt_id]
            )
            .await
            .is_err()
    );
    assert!(
        runtime
            .execute(
                "DELETE FROM workflow_task_quality WHERE attempt_id=$1",
                &[&first.attempt_id]
            )
            .await
            .is_err()
    );
    runtime_driver.abort();
    driver.abort();
    Ok(())
}

async fn verify_quality_receipts(
    store: &WorkflowStore,
    task: &ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
    uid: &str,
    quality: &QualityOutcome,
) -> Result {
    let report = W::QualityReport {
        outcome: quality.clone(),
    };
    assert!(
        store
            .task_worker(&task.attempt_id, 1, uid, &report)
            .await?
            .authorized
    );
    assert!(
        store
            .task_worker(&task.attempt_id, 1, uid, &report)
            .await?
            .authorized
    );
    assert!(
        store
            .task_worker(&task.attempt_id, 2, uid, &report)
            .await
            .is_err()
    );
    assert!(
        store
            .task_worker(&task.attempt_id, 1, &key(), &report)
            .await
            .is_err()
    );
    let mut changed = quality.clone();
    changed.snapshot_digest = format!("sha256:{}", "f".repeat(64));
    assert!(
        store
            .task_worker(
                &task.attempt_id,
                1,
                uid,
                &W::QualityReport { outcome: changed }
            )
            .await
            .is_err()
    );
    let mut changed = quality.clone();
    changed.safe_reason = "runtime_unavailable".into();
    assert!(
        store
            .task_worker(
                &task.attempt_id,
                1,
                uid,
                &W::QualityReport { outcome: changed }
            )
            .await
            .is_err()
    );
    let invalid = QualityOutcome {
        state: "passed".into(),
        ..quality.clone()
    };
    assert!(
        store
            .task_worker(
                &task.attempt_id,
                1,
                uid,
                &W::QualityReport { outcome: invalid }
            )
            .await
            .is_err()
    );
    Ok(())
}

async fn prove_quality_rollback(
    store: &WorkflowStore,
    task: &ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
    uid: &str,
    quality: &QualityOutcome,
    admin: &str,
) -> Result {
    assert!(
        quality.validate(),
        "rollback fixture must satisfy the current quality contract"
    );
    let (db, driver) = connect(admin).await?;
    let before: i64 = db
        .query_one("SELECT count(*) FROM workflow_updates", &[])
        .await?
        .get(0);
    db.batch_execute("CREATE FUNCTION fixture_reject_quality() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture'; END $$; CREATE TRIGGER fixture_reject_quality BEFORE INSERT ON workflow_task_quality FOR EACH ROW EXECUTE FUNCTION fixture_reject_quality()").await?;
    let result = store
        .task_worker(
            &task.attempt_id,
            1,
            uid,
            &W::QualityReport {
                outcome: quality.clone(),
            },
        )
        .await;
    db.batch_execute("DROP TRIGGER fixture_reject_quality ON workflow_task_quality; DROP FUNCTION fixture_reject_quality()").await?;
    assert!(matches!(result, Err(StoreError::Database(_))));
    let after: i64 = db
        .query_one("SELECT count(*) FROM workflow_updates", &[])
        .await?
        .get(0);
    assert_eq!(before, after);
    assert!(
        !db.query_one(
            "SELECT EXISTS(SELECT 1 FROM workflow_task_quality WHERE attempt_id=$1)",
            &[&task.attempt_id]
        )
        .await?
        .get::<_, bool>(0)
    );
    db.batch_execute("CREATE FUNCTION fixture_reject_feedback() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture'; END $$; CREATE TRIGGER fixture_reject_feedback BEFORE INSERT ON workflow_task_feedback FOR EACH ROW EXECUTE FUNCTION fixture_reject_feedback()").await?;
    let failed_feedback = store
        .task_worker(
            &task.attempt_id,
            1,
            uid,
            &W::QualityReport {
                outcome: quality.clone(),
            },
        )
        .await;
    db.batch_execute("DROP TRIGGER fixture_reject_feedback ON workflow_task_feedback; DROP FUNCTION fixture_reject_feedback()").await?;
    assert!(matches!(failed_feedback, Err(StoreError::Database(_))));
    assert!(
        !db.query_one(
            "SELECT EXISTS(SELECT 1 FROM workflow_task_quality WHERE attempt_id=$1)",
            &[&task.attempt_id]
        )
        .await?
        .get::<_, bool>(0)
    );
    let after: i64 = db
        .query_one("SELECT count(*) FROM workflow_updates", &[])
        .await?
        .get(0);
    assert_eq!(
        before, after,
        "feedback failure rolls back quality and observation"
    );
    driver.abort();
    Ok(())
}

fn failed_quality(
    command: &ayni_orchestrator_events::task_execution::TaskCommand,
    outcome: &TaskOutcome,
) -> QualityOutcome {
    let hash = format!("sha256:{}", "e".repeat(64));
    let artifact = serde_json::json!({
        "schema_version":"0.5.0", "invocation":{"command":"check","scope":{}}, "execution_mode":"managed",
        "environment_lock_fingerprint":hash,"source_fingerprint":hash,"contract_digest":hash,
        "prebuilt_runtime":{"metadata_digest":hash,"certificate":{"certificate":{"platform":"linux/arm64","lock_fingerprint":hash}}},
        "tool_versions":[{"tool":"rust","version":"1.94.0"}],
        "completion":{"scope":"repository","state":"complete","expected_targets":1,"completed_targets":1,"detected_targets":1,"skipped_targets":0,"issues":[]},
        "aggregate":{"status":"fail","total_rows":1,"passing_rows":0,"failing_rows":1},"rows":[{"pass":false}]
    }).to_string();
    QualityOutcome {
        state: "failed".into(),
        safe_reason: "quality_failed".into(),
        snapshot_digest: snapshot_digest(command, outcome),
        lock_digest: command.lock_digest.clone(),
        runtime_image: command.runtime_image.clone(),
        runtime_platform: "linux/arm64".into(),
        runtime_metadata_digest: hash.clone(),
        executor_digest: hash.clone(),
        contract_digest: hash.clone(),
        lock_fingerprint: hash.clone(),
        source_fingerprint: hash.clone(),
        validation_input_digest: hash,
        exit_code: Some(1),
        artifact_digest: ayni_orchestrator_events::task_execution::digest(artifact.as_bytes()),
        artifact,
        expected_targets: 1,
        completed_targets: 1,
    }
}

async fn cancel_queued_task(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    queued_cancel: &ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
) -> Result {
    let before = store
        .list_task_feedback(scope.workspace, scope.project, scope.conversation, "", 32)
        .await?;
    let cancel_queued = key();
    let cancelled = store
        .cancel_task_execution(
            scope,
            &queued_cancel.task_id,
            &queued_cancel.attempt_id,
            &cancel_queued,
        )
        .await?;
    assert_eq!(
        store
            .cancel_task_execution(
                scope,
                &queued_cancel.task_id,
                &queued_cancel.attempt_id,
                &cancel_queued
            )
            .await?,
        cancelled
    );
    assert_eq!(
        store
            .get_task_run(scope, &queued_cancel.task_id, &queued_cancel.attempt_id)
            .await?
            .state,
        "cancelled"
    );
    assert_eq!(
        before,
        store
            .list_task_feedback(scope.workspace, scope.project, scope.conversation, "", 32)
            .await?
    );
    Ok(())
}

async fn cancel_active_call(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    first: &ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
    command: &ayni_orchestrator_events::task_execution::TaskCommand,
    uid: &str,
) -> std::result::Result<TaskOutcome, Box<dyn Error + Send + Sync>> {
    let sending = W::Sending {
        request_digest: format!("sha256:{}", "d".repeat(64)),
    };
    assert!(
        store
            .task_worker(&first.attempt_id, 1, uid, &sending)
            .await?
            .authorized
    );
    assert!(
        !store
            .task_worker(&first.attempt_id, 1, uid, &sending)
            .await?
            .authorized,
        "first consumed paid-call permit cannot be replayed"
    );
    let cancel_key = key();
    let cancel = store
        .cancel_task_execution(scope, &first.task_id, &first.attempt_id, &cancel_key)
        .await?;
    assert_eq!(
        store
            .cancel_task_execution(scope, &first.task_id, &first.attempt_id, &cancel_key)
            .await?,
        cancel
    );
    let outcome = TaskOutcome {
        state: "completed".into(),
        patch: "diff --git a/a b/a\n".into(),
        ..Default::default()
    };
    assert!(matches!(
        store
            .task_worker(
                &first.attempt_id,
                1,
                uid,
                &W::Report {
                    outcome: outcome.clone()
                }
            )
            .await,
        Err(StoreError::TaskExecutionConflict)
    ));
    let stale_quality = QualityOutcome {
        state: "blocked".into(),
        safe_reason: "fixture_only".into(),
        snapshot_digest: snapshot_digest(command, &outcome),
        lock_digest: command.lock_digest.clone(),
        runtime_image: command.runtime_image.clone(),
        ..Default::default()
    };
    assert!(
        store
            .task_worker(
                &first.attempt_id,
                1,
                uid,
                &W::QualityReport {
                    outcome: stale_quality
                }
            )
            .await
            .is_err()
    );
    let held = store
        .task_supervisor(&S::Poll)
        .await?
        .ok_or("held slot missing")?;
    assert_eq!(held.command.attempt_id, first.attempt_id);
    assert_eq!(held.state, "cancelling");
    assert_eq!(held.provider_state, "uncertain");
    assert!(
        store
            .task_supervisor(&S::Stopped {
                attempt_id: first.attempt_id.clone(),
                generation: 1,
                pod_uid: key()
            })
            .await
            .is_err()
    );
    store
        .task_supervisor(&S::Stopped {
            attempt_id: first.attempt_id.clone(),
            generation: 1,
            pod_uid: uid.to_owned(),
        })
        .await?;
    Ok(outcome)
}

async fn store_fixture()
-> std::result::Result<(String, String, WorkflowStore), Box<dyn Error + Send + Sync>> {
    let admin = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let runtime = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    migrate_for_runtime(
        &admin,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let store = WorkflowStore::new(runtime.clone());
    Ok((admin, runtime, store))
}

async fn prove_failed_and_interrupted_feedback(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    tasks: &[ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation],
) -> Result {
    for (index, task) in tasks.iter().enumerate() {
        store
            .start_task_execution(
                scope,
                &task.task_id,
                &task.attempt_id,
                &key(),
                b"fixture-pack",
            )
            .await?;
        store.task_supervisor(&S::Poll).await?;
        let uid = key();
        store
            .task_supervisor(&S::Observe {
                attempt_id: task.attempt_id.clone(),
                generation: 1,
                pod_uid: uid.clone(),
            })
            .await?;
        if index == 0 {
            let report = W::Report {
                outcome: TaskOutcome {
                    state: "failed".into(),
                    safe_reason: "source_context_limit".into(),
                    ..Default::default()
                },
            };
            store
                .task_worker(&task.attempt_id, 1, &uid, &report)
                .await?;
            store
                .task_worker(&task.attempt_id, 1, &uid, &report)
                .await?;
        }
        if index >= 2 {
            let outcome = TaskOutcome {
                state: "completed".into(),
                ..Default::default()
            };
            store
                .task_worker(
                    &task.attempt_id,
                    1,
                    &uid,
                    &W::Report {
                        outcome: outcome.clone(),
                    },
                )
                .await?;
            if index == 2 {
                let command = store
                    .get_task_run(scope, &task.task_id, &task.attempt_id)
                    .await?
                    .command;
                let quality = QualityOutcome {
                    state: "blocked".into(),
                    safe_reason: "runtime_unavailable".into(),
                    snapshot_digest: snapshot_digest(&command, &outcome),
                    lock_digest: command.lock_digest,
                    runtime_image: command.runtime_image,
                    ..Default::default()
                };
                store
                    .task_worker(
                        &task.attempt_id,
                        1,
                        &uid,
                        &W::QualityReport { outcome: quality },
                    )
                    .await?;
            }
        }
        let stopped = S::Stopped {
            attempt_id: task.attempt_id.clone(),
            generation: 1,
            pod_uid: uid,
        };
        store.task_supervisor(&stopped).await?;
        store.task_supervisor(&stopped).await?;
    }
    Ok(())
}

async fn prove_feedback_scope_pagination_and_retention(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    wrong: PlanScope<'_>,
    runtime: &str,
) -> Result {
    let page = store
        .list_task_feedback(scope.workspace, scope.project, scope.conversation, "", 32)
        .await?;
    assert_eq!(
        page.feedback.len(),
        7,
        "execution and quality attention survive each accepted failure boundary"
    );
    let mut reasons: Vec<_> = page
        .feedback
        .iter()
        .map(|v| v.safe_reason.as_str())
        .collect();
    reasons.sort_unstable();
    assert_eq!(
        reasons,
        vec![
            "execution_deadline_exceeded",
            "execution_failed",
            "execution_interrupted",
            "provider_uncertain",
            "quality_blocked",
            "quality_failed",
            "quality_interrupted"
        ]
    );
    assert!(
        page.feedback
            .iter()
            .all(|v| v.conversation_id == scope.conversation && !v.summary.is_empty())
    );
    assert_eq!(
        page,
        store
            .list_task_feedback(scope.workspace, scope.project, "", "", 32)
            .await?
    );
    assert!(
        store
            .list_task_feedback(wrong.workspace, wrong.project, wrong.conversation, "", 32)
            .await?
            .feedback
            .is_empty()
    );
    let reopened = WorkflowStore::new(runtime.to_owned());
    assert_eq!(
        page,
        reopened
            .list_task_feedback(scope.workspace, scope.project, scope.conversation, "", 32)
            .await?
    );
    let mut ids = Vec::new();
    let mut after = String::new();
    loop {
        let part = store
            .list_task_feedback(scope.workspace, scope.project, "", &after, 1)
            .await?;
        ids.extend(part.feedback.into_iter().map(|v| v.id));
        if part.next_after_id.is_empty() {
            break;
        }
        after = part.next_after_id;
    }
    assert_eq!(
        ids,
        page.feedback
            .iter()
            .map(|v| v.id.clone())
            .collect::<Vec<_>>()
    );
    prove_feedback_read_bounds(store, scope).await?;
    let (db, driver) = connect(runtime).await?;
    assert!(
        db.execute(
            "UPDATE workflow_task_feedback SET safe_reason='execution_failed' WHERE id=$1",
            &[&ids[0]]
        )
        .await
        .is_err()
    );
    assert!(
        db.execute("DELETE FROM workflow_task_feedback WHERE id=$1", &[&ids[0]])
            .await
            .is_err()
    );
    driver.abort();
    Ok(())
}

async fn prove_feedback_read_bounds(store: &WorkflowStore, scope: PlanScope<'_>) -> Result {
    assert!(matches!(
        store
            .list_task_feedback(scope.workspace, scope.project, "", "", 33)
            .await,
        Err(StoreError::InvalidInput("limit"))
    ));
    assert!(matches!(
        store
            .list_task_feedback(scope.workspace, scope.project, "", "invalid", 20)
            .await,
        Err(StoreError::InvalidInput("after_id"))
    ));
    assert!(matches!(
        store
            .list_task_feedback(scope.workspace, &key(), "", "", 20)
            .await,
        Err(StoreError::NotFound)
    ));
    let other_workspace = store
        .create_workspace(&key(), "Other workspace")
        .await?
        .resource;
    assert!(matches!(
        store
            .list_task_feedback(&other_workspace.id, scope.project, "", "", 20)
            .await,
        Err(StoreError::NotFound)
    ));
    let another = store
        .create_project(&key(), scope.workspace, "Other project", "Isolation")
        .await?
        .resource;
    assert!(matches!(
        store
            .list_task_feedback(scope.workspace, &another.id, scope.conversation, "", 20)
            .await,
        Err(StoreError::NotFound)
    ));
    Ok(())
}

async fn prove_retained_review(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    wrong: PlanScope<'_>,
    preparations: &[ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation],
    runtime: &str,
) -> Result {
    prove_failed_and_interrupted_feedback(store, scope, &preparations[4..]).await?;
    recovery::prove(
        store,
        scope,
        wrong,
        &preparations[0],
        &preparations[1],
        runtime,
    )
    .await?;
    prove_feedback_scope_pagination_and_retention(store, scope, wrong, runtime).await
}
