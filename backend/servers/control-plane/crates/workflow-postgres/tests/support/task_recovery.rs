use super::{Result, key};
use ayni_orchestrator_workflow_postgres::store::task_recovery::{
    ProposalInput, RecoveryContent as C, RecoveryInput as I,
};
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, connect, execution::TaskExecutionPreparation, plans::PlanScope,
};

pub async fn prove(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    wrong: PlanScope<'_>,
    uncertain: &TaskExecutionPreparation,
    quality: &TaskExecutionPreparation,
    runtime: &str,
) -> Result {
    proposal(store, scope, uncertain, true).await?;
    proposal(store, scope, quality, false).await?;
    let task = &quality.task_id;
    let attempt = &quality.attempt_id;
    assert!(matches!(
        store.list_task_recovery(wrong, task, attempt, "", 20).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.list_task_recovery(scope, task, attempt, "", 33).await,
        Err(StoreError::InvalidInput(_))
    ));
    let question = I::Question {
        text: "Should the existing accepted change be retained?".into(),
    };
    assert!(matches!(
        store
            .record_task_recovery(wrong, task, attempt, &key(), &question)
            .await,
        Err(StoreError::NotFound)
    ));
    let command = key();
    let (a, b) = tokio::join!(
        store.record_task_recovery(scope, task, attempt, &command, &question),
        store.record_task_recovery(scope, task, attempt, &command, &question)
    );
    let q = a?;
    assert_eq!(q, b?);
    assert!(matches!(
        store
            .record_task_recovery(
                scope,
                task,
                attempt,
                &command,
                &I::Question {
                    text: "changed".into()
                }
            )
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    let answer = I::Answer {
        question_id: q.id.clone(),
        text: "Keep the accepted change; investigate the failed check.".into(),
    };
    let first_key = key();
    let second_key = key();
    let (a, b) = tokio::join!(
        store.record_task_recovery(scope, task, attempt, &first_key, &answer),
        store.record_task_recovery(scope, task, attempt, &second_key, &answer)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(
        store
            .record_task_recovery(
                scope,
                task,
                attempt,
                &key(),
                &I::Answer {
                    question_id: attempt.clone(),
                    text: "foreign question".into()
                }
            )
            .await,
        Err(StoreError::NotFound)
    ));
    retention(store, scope, task, attempt, runtime, &q.id).await
}
async fn retention(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    task: &str,
    attempt: &str,
    runtime: &str,
    question_id: &str,
) -> Result {
    let mut records = Vec::new();
    let mut after = String::new();
    loop {
        let page = store
            .list_task_recovery(scope, task, attempt, &after, 1)
            .await?;
        records.extend(page.records);
        after = page.next_after_id;
        if after.is_empty() {
            break;
        }
    }
    assert_eq!(records.len(), 3);
    let reopened = WorkflowStore::new(runtime.to_owned());
    assert_eq!(
        records,
        reopened
            .list_task_recovery(scope, task, attempt, "", 32)
            .await?
            .records
    );
    let (db, driver) = connect(runtime).await?;
    assert!(
        db.execute(
            "UPDATE workflow_task_recovery SET content=content WHERE id=$1",
            &[&question_id]
        )
        .await
        .is_err()
    );
    assert!(
        db.execute(
            "DELETE FROM workflow_task_recovery WHERE id=$1",
            &[&question_id]
        )
        .await
        .is_err()
    );
    driver.abort();
    Ok(())
}
async fn proposal(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    task: &TaskExecutionPreparation,
    uncertain: bool,
) -> Result {
    let before = store
        .get_task_run(scope, &task.task_id, &task.attempt_id)
        .await?;
    let feedback = store
        .list_task_feedback(scope.workspace, scope.project, scope.conversation, "", 32)
        .await?
        .feedback
        .into_iter()
        .find(|f| f.attempt_id == task.attempt_id)
        .ok_or("feedback missing")?;
    let mut input = ProposalInput {
        feedback_id: feedback.id,
        explanation: "Inspect retained work before a bounded recovery.".into(),
        action: "continue".into(),
        additional_seconds: 300,
        additional_tokens: 4096,
    };
    let command = key();
    let request = I::Proposal(input.clone());
    let record = store
        .record_task_recovery(scope, &task.task_id, &task.attempt_id, &command, &request)
        .await?;
    assert_eq!(
        record,
        store
            .record_task_recovery(scope, &task.task_id, &task.attempt_id, &command, &request)
            .await?
    );
    let C::Proposal(p) = &record.content else {
        return Err("proposal missing".into());
    };
    assert_eq!(p.evidence.provider_uncertain, uncertain);
    assert_eq!(
        p.evidence.patch_available,
        before.outcome.as_ref().is_some_and(|o| !o.patch.is_empty())
    );
    assert_eq!(p.evidence.source_commit, before.command.source_commit);
    if !uncertain {
        assert_eq!(p.evidence.quality_state, "failed");
        assert!(!p.evidence.quality_digest.is_empty());
    }
    input.additional_seconds = 0;
    assert!(matches!(
        store
            .record_task_recovery(
                scope,
                &task.task_id,
                &task.attempt_id,
                &key(),
                &I::Proposal(input.clone())
            )
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    input.additional_seconds = 301;
    assert!(matches!(
        store
            .record_task_recovery(
                scope,
                &task.task_id,
                &task.attempt_id,
                &command,
                &I::Proposal(input)
            )
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert_eq!(
        before,
        store
            .get_task_run(scope, &task.task_id, &task.attempt_id)
            .await?
    );
    Ok(())
}
