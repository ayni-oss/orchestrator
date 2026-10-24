use super::{Error, PlanScope, S, WorkflowStore};

pub async fn claim(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    first: &ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
    second: &ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
) -> std::result::Result<
    ayni_orchestrator_events::task_execution::TaskRun,
    Box<dyn Error + Send + Sync>,
> {
    let (one_claim, other_claim) = tokio::join!(
        store.task_supervisor(&S::Poll),
        store.task_supervisor(&S::Poll)
    );
    let one = one_claim?.ok_or("claim missing")?;
    let other = other_claim?.ok_or("other claim missing")?;
    assert_ne!(one.command.attempt_id, other.command.attempt_id);
    let owned = if one.command.attempt_id == first.attempt_id {
        one
    } else {
        other
    };
    assert_eq!(owned.command.attempt_id, first.attempt_id);
    assert_eq!(
        store
            .get_task_run(scope, &second.task_id, &second.attempt_id)
            .await?
            .state,
        "claimed"
    );
    Ok(owned)
}
