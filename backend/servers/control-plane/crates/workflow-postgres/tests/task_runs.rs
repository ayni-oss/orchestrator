//! Serial task ownership, fencing, exact command receipts and retained result acceptance.
use ayni_orchestrator_events::task_execution::{
    SupervisorRequest as S, TaskOutcome, WorkerRequest as W,
};
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, connect,
    execution::ExecutionInputs,
    migrate_for_runtime,
    plans::{PlanContent, PlanScope, PlanTaskSpecification},
};
use std::error::Error;
use uuid::Uuid;
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
    let admin = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let runtime = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    migrate_for_runtime(
        &admin,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let store = WorkflowStore::new(runtime.clone());
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
        tasks: (0..4)
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
    let uid = key();
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
    let sending = W::Sending {
        request_digest: format!("sha256:{}", "d".repeat(64)),
    };
    assert!(
        store
            .task_worker(&first.attempt_id, 1, &uid, &sending)
            .await?
            .authorized
    );
    assert!(
        !store
            .task_worker(&first.attempt_id, 1, &uid, &sending)
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
                &uid,
                &W::Report {
                    outcome: outcome.clone()
                }
            )
            .await,
        Err(StoreError::TaskExecutionConflict)
    ));
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
            pod_uid: uid,
        })
        .await?;
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
        .await
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
    runtime_driver.abort();
    driver.abort();
    Ok(())
}
