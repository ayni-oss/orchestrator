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
    prove_child(store, scope, task, &before, &receipt, uncertain, runtime).await
}
async fn prove_child(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    task: &TaskExecutionPreparation,
    before: &ayni_orchestrator_events::task_execution::TaskRun,
    receipt: &ayni_orchestrator_workflow_postgres::store::task_recovery::RecoveryConfirmation,
    uncertain: bool,
    runtime: &str,
) -> Result {
    let child = store
        .get_task_run(scope, &task.task_id, &receipt.attempt_id)
        .await?;
    prove_project_fact(store, scope, task, receipt, runtime).await?;
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
    prove_resolution(store, scope, task, receipt, &after).await?;
    store
        .cancel_task_execution(scope, &task.task_id, &receipt.attempt_id, &key())
        .await?;
    let feedback = store
        .list_task_feedback(scope.workspace, scope.project, scope.conversation, "", 32)
        .await?;
    assert_eq!(
        feedback
            .feedback
            .iter()
            .find(|f| f.attempt_id == task.attempt_id)
            .ok_or("feedback missing")?
            .recovery_state,
        "cancelled"
    );
    assert!(
        store
            .task_supervisor(&SupervisorRequest::Poll)
            .await?
            .is_none()
    );
    Ok(())
}
async fn prove_resolution(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    task: &TaskExecutionPreparation,
    receipt: &ayni_orchestrator_workflow_postgres::store::task_recovery::RecoveryConfirmation,
    after: &ayni_orchestrator_workflow_postgres::store::task_recovery::RecoveryPage,
) -> Result {
    let last_page = store
        .list_task_recovery(
            scope,
            &task.task_id,
            &task.attempt_id,
            &after.records.last().ok_or("record missing")?.id,
            1,
        )
        .await?;
    assert!(last_page.records.is_empty());
    assert_eq!(
        last_page.confirmations,
        vec![receipt.clone()],
        "parent consent must be visible on every page"
    );
    let feedback = store
        .list_task_feedback(scope.workspace, scope.project, scope.conversation, "", 32)
        .await?;
    let resolved = feedback
        .feedback
        .iter()
        .find(|f| f.attempt_id == task.attempt_id)
        .ok_or("feedback missing")?;
    assert_eq!(resolved.recovery_attempt_id, receipt.attempt_id);
    assert_eq!(resolved.recovery_state, "queued");
    assert!(resolved.recovery_quality_state.is_empty());
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

async fn prove_project_fact(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    task: &TaskExecutionPreparation,
    receipt: &ayni_orchestrator_workflow_postgres::store::task_recovery::RecoveryConfirmation,
    runtime: &str,
) -> Result {
    let fresh = store
        .create_conversation(&key(), scope.workspace, scope.project, "Recovery facts")
        .await?
        .resource;
    let turn = store
        .send_message(
            &key(),
            scope.workspace,
            scope.project,
            &fresh.id,
            "Inspect recovery facts",
        )
        .await?;
    let (client, driver) = ayni_orchestrator_workflow_postgres::connect(runtime).await?;
    let bytes: Vec<u8> = client
        .query_one(
            "SELECT exact_command_bytes FROM workflow_dispatch_intents WHERE turn_id=$1",
            &[&turn.turn_id],
        )
        .await?
        .get(0);
    let (_, _, payload) = ayni_orchestrator_events::workflow::decode_command(&bytes)?;
    let facts = payload.project_status.ok_or("recovery project facts")?;
    let fact = facts
        .tasks
        .iter()
        .find(|fact| fact.task_id == task.task_id)
        .ok_or("recovery task fact")?;
    assert_eq!(fact.attempt_id, receipt.attempt_id);
    assert_eq!(fact.parent_attempt_id, task.attempt_id);
    assert_eq!(fact.coding_state, "queued");
    assert_eq!(fact.provider_state, "not_started");
    assert!(fact.accepted_coding_state.is_empty());
    assert!(fact.quality_state.is_empty());
    assert_eq!(
        facts
            .tasks
            .iter()
            .filter(|fact| fact.task_id == task.task_id)
            .count(),
        1
    );
    driver.abort();
    Ok(())
}
