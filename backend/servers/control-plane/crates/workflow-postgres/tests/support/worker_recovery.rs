use super::{PlanScope, Result, S, StoreError, WorkflowStore, key};

pub async fn prove(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    task: &ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
) -> Result {
    use ayni_orchestrator_workflow_postgres::store::task_recovery::{
        ConfirmRecovery, RecoveryContent, RecoveryInput,
    };
    let records = store
        .list_task_recovery(scope, &task.task_id, &task.attempt_id, "", 32)
        .await?
        .records;
    assert_eq!(
        records.len(),
        2,
        "stopped/report replay cannot duplicate model review records"
    );
    assert!(records.iter().all(|record| record.author == "agent"));
    let question = records
        .iter()
        .find(|record| record.kind == "question")
        .ok_or("worker question missing")?;
    let proposal = records
        .iter()
        .find(|record| record.kind == "proposal")
        .ok_or("model proposal missing")?;
    let command = key();
    let request = ConfirmRecovery {
        proposal_id: &proposal.id,
        proposal_digest: &proposal.content_digest,
        key: &command,
        acknowledge_uncertainty: false,
    };
    assert!(matches!(
        store
            .confirm_task_recovery(scope, &task.task_id, &request)
            .await,
        Err(StoreError::TaskExecutionConflict)
    ));
    assert!(matches!(
        store
            .record_task_recovery(
                scope,
                &task.task_id,
                &task.attempt_id,
                "worker:forged",
                &RecoveryInput::Question {
                    text: "forged".into()
                }
            )
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    let answer = "Retain the exact original fixture.";
    store
        .record_task_recovery(
            scope,
            &task.task_id,
            &task.attempt_id,
            &key(),
            &RecoveryInput::Answer {
                question_id: question.id.clone(),
                text: answer.into(),
            },
        )
        .await?;
    let confirmation = store
        .confirm_task_recovery(scope, &task.task_id, &request)
        .await?;
    let child = store
        .get_task_run(scope, &task.task_id, &confirmation.attempt_id)
        .await?;
    let recovery = child.command.recovery.ok_or("recovery missing")?;
    assert_eq!(recovery.answers.len(), 1);
    assert_eq!(recovery.answers[0].question_id, question.id);
    assert_eq!(recovery.answers[0].answer, answer);
    let RecoveryContent::Proposal(proposal) = &proposal.content else {
        return Err("proposal missing".into());
    };
    assert_eq!(child.command.max_tokens, proposal.request.additional_tokens);
    assert_eq!(
        child.command.max_seconds,
        proposal.request.additional_seconds
    );
    store
        .cancel_task_execution(scope, &task.task_id, &confirmation.attempt_id, &key())
        .await?;
    assert!(
        store.task_supervisor(&S::Poll).await?.is_none(),
        "review evidence must leave no runnable fixture work"
    );
    Ok(())
}
