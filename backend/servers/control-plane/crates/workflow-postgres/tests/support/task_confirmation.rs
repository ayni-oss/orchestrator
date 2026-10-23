use super::{Result, key};
use ayni_orchestrator_events::task_execution::{SupervisorRequest, WorkerRequest};
use ayni_orchestrator_workflow_postgres::store::task_recovery::{
    ConfirmRecovery, ProposalInput, RecoveryInput, RecoveryRecord,
};
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, execution::TaskExecutionPreparation, plans::PlanScope,
};

pub async fn prove(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    wrong: PlanScope<'_>,
    tasks: &[TaskExecutionPreparation],
    runtime: &str,
) -> Result {
    Box::pin(confirm(store, scope, wrong, &tasks[1], runtime, false)).await?;
    Box::pin(confirm(store, scope, wrong, &tasks[0], runtime, true)).await
}
async fn confirm(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    wrong: PlanScope<'_>,
    task: &TaskExecutionPreparation,
    runtime: &str,
    uncertain: bool,
) -> Result {
    let before = store
        .get_task_run(scope, &task.task_id, &task.attempt_id)
        .await?;
    let records = store
        .list_task_recovery(scope, &task.task_id, &task.attempt_id, "", 32)
        .await?
        .records;
    let proposal = records
        .iter()
        .find(|r| r.kind == "proposal")
        .ok_or("proposal missing")?;
    let first_key = key();
    let second_key = key();
    let request = ConfirmRecovery {
        proposal_id: &proposal.id,
        proposal_digest: &proposal.content_digest,
        key: &first_key,
        acknowledge_uncertainty: uncertain,
    };
    reject_invalid(store, scope, wrong, task, &request, uncertain).await?;
    let alternate = alternate(store, scope, task, proposal).await?;
    if !uncertain {
        atomicity(store, scope, task, &request).await?;
    }
    let second = ConfirmRecovery {
        key: &second_key,
        ..request
    };
    let (a, b) = tokio::join!(
        store.confirm_task_recovery(scope, &task.task_id, &request),
        store.confirm_task_recovery(scope, &task.task_id, &second)
    );
    let receipt = a?;
    assert_eq!(receipt, b?);
    assert_ne!(receipt.attempt_id, task.attempt_id);
    let reopened = WorkflowStore::new(runtime.to_owned());
    assert_eq!(
        receipt,
        reopened
            .confirm_task_recovery(scope, &task.task_id, &request)
            .await?
    );
    assert_eq!(
        receipt,
        reopened
            .confirm_task_recovery(scope, &task.task_id, &second)
            .await?
    );
    let stale = ConfirmRecovery {
        proposal_id: &alternate.id,
        proposal_digest: &alternate.content_digest,
        key: &key(),
        acknowledge_uncertainty: uncertain,
    };
    assert!(matches!(
        store
            .confirm_task_recovery(scope, &task.task_id, &stale)
            .await,
        Err(StoreError::TaskExecutionConflict)
    ));
    assert!(matches!(
        store
            .confirm_task_recovery(
                scope,
                &task.task_id,
                &ConfirmRecovery {
                    key: &second_key,
                    ..stale
                }
            )
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    prove_child(store, scope, task, &before, &receipt, uncertain).await
}
async fn prove_child(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    task: &TaskExecutionPreparation,
    before: &ayni_orchestrator_events::task_execution::TaskRun,
    receipt: &ayni_orchestrator_workflow_postgres::store::task_recovery::RecoveryConfirmation,
    uncertain: bool,
) -> Result {
    let child = store
        .get_task_run(scope, &task.task_id, &receipt.attempt_id)
        .await?;
    assert_eq!(child.state, "queued");
    assert_eq!(child.provider_state, "not_started");
    assert_eq!(child.command.max_seconds, 300);
    assert_eq!(child.command.max_tokens, 4096);
    assert_eq!(
        child.command.source_pack_digest,
        before.command.source_pack_digest
    );
    assert_eq!(
        child.command.configuration_revision_id,
        before.command.configuration_revision_id
    );
    assert_eq!(
        child
            .command
            .recovery
            .as_ref()
            .ok_or("link missing")?
            .parent_attempt_id,
        task.attempt_id
    );
    if !uncertain {
        assert_eq!(
            child
                .command
                .recovery
                .as_ref()
                .ok_or("seed missing")?
                .seed_patch,
            before.outcome.as_ref().ok_or("patch missing")?.patch
        );
    }
    assert_eq!(
        *before,
        store
            .get_task_run(scope, &task.task_id, &task.attempt_id)
            .await?
    );
    assert!(
        store
            .task_worker(
                &task.attempt_id,
                before.generation,
                &before.pod_uid,
                &WorkerRequest::Input
            )
            .await
            .is_err()
    );
    assert_eq!(
        store
            .get_task_execution(scope, &task.task_id)
            .await?
            .ok_or("latest missing")?
            .attempt_id,
        receipt.attempt_id
    );
    let after = store
        .list_task_recovery(scope, &task.task_id, &task.attempt_id, "", 32)
        .await?;
    assert_eq!(after.confirmations, vec![receipt.clone()]);
    store
        .cancel_task_execution(scope, &task.task_id, &receipt.attempt_id, &key())
        .await?;
    assert!(
        store
            .task_supervisor(&SupervisorRequest::Poll)
            .await?
            .is_none()
    );
    Ok(())
}
async fn alternate(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    task: &TaskExecutionPreparation,
    record: &RecoveryRecord,
) -> std::result::Result<RecoveryRecord, Box<dyn std::error::Error + Send + Sync>> {
    let ayni_orchestrator_workflow_postgres::store::task_recovery::RecoveryContent::Proposal(p) =
        &record.content
    else {
        return Err("proposal missing".into());
    };
    store
        .record_task_recovery(
            scope,
            &task.task_id,
            &task.attempt_id,
            &key(),
            &RecoveryInput::Proposal(ProposalInput {
                explanation: "Alternative action for the same attempt".into(),
                ..p.request.clone()
            }),
        )
        .await
        .map_err(Into::into)
}

async fn atomicity(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    task: &TaskExecutionPreparation,
    request: &ConfirmRecovery<'_>,
) -> Result {
    let (admin, driver) = ayni_orchestrator_workflow_postgres::connect(&std::env::var(
        "AYNI_POSTGRES_FEASIBILITY_URL",
    )?)
    .await?;
    admin
        .execute(
            "UPDATE workflow_task_runs SET generation=generation+1 WHERE attempt_id=$1",
            &[&task.attempt_id],
        )
        .await?;
    let stale = store
        .confirm_task_recovery(scope, &task.task_id, request)
        .await;
    admin
        .execute(
            "UPDATE workflow_task_runs SET generation=generation-1 WHERE attempt_id=$1",
            &[&task.attempt_id],
        )
        .await?;
    assert!(matches!(stale, Err(StoreError::TaskExecutionConflict)));
    admin.batch_execute("CREATE FUNCTION fixture_reject_confirmation() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture'; END $$; CREATE TRIGGER fixture_reject_confirmation BEFORE INSERT ON workflow_task_recovery_confirmation_receipts FOR EACH ROW EXECUTE FUNCTION fixture_reject_confirmation()").await?;
    let result = store
        .confirm_task_recovery(scope, &task.task_id, request)
        .await;
    admin.batch_execute("DROP TRIGGER fixture_reject_confirmation ON workflow_task_recovery_confirmation_receipts; DROP FUNCTION fixture_reject_confirmation()").await?;
    assert!(matches!(result, Err(StoreError::Database(_))));
    assert_eq!(
        store
            .get_task_execution(scope, &task.task_id)
            .await?
            .ok_or("preparation missing")?
            .attempt_id,
        task.attempt_id
    );
    assert!(
        store
            .list_task_recovery(scope, &task.task_id, &task.attempt_id, "", 32)
            .await?
            .confirmations
            .is_empty()
    );
    driver.abort();
    Ok(())
}

async fn reject_invalid(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    wrong: PlanScope<'_>,
    task: &TaskExecutionPreparation,
    request: &ConfirmRecovery<'_>,
    uncertain: bool,
) -> Result {
    assert!(matches!(
        store
            .confirm_task_recovery(wrong, &task.task_id, request)
            .await,
        Err(StoreError::NotFound)
    ));
    let wrong_digest = format!("sha256:{}", "0".repeat(64));
    assert!(matches!(
        store
            .confirm_task_recovery(
                scope,
                &task.task_id,
                &ConfirmRecovery {
                    proposal_digest: &wrong_digest,
                    ..*request
                }
            )
            .await,
        Err(StoreError::PlanDigestMismatch)
    ));
    if uncertain {
        assert!(matches!(
            store
                .confirm_task_recovery(
                    scope,
                    &task.task_id,
                    &ConfirmRecovery {
                        acknowledge_uncertainty: false,
                        ..*request
                    }
                )
                .await,
            Err(StoreError::InvalidInput(_))
        ));
    }
    Ok(())
}
