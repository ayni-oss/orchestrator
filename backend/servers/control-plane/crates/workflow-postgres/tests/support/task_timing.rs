//! Accepted task timing boundaries remain ordered and scoped.
use super::*;

pub async fn prove(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    wrong: PlanScope<'_>,
    preparation: &ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
    max_seconds: u32,
) -> Result {
    let timing = store
        .get_task_timing(
            scope,
            &preparation.task_id,
            &preparation.attempt_id,
            max_seconds,
        )
        .await?;
    let boundaries = [
        timing.queue_accepted_at,
        timing.claim_accepted_at,
        timing.coding_result_accepted_at,
        timing.quality_result_accepted_at,
        timing.execution_stopped_at,
        timing.observed_at,
    ];
    let mut previous = (i64::MIN, 0);
    for boundary in boundaries {
        let boundary = boundary.ok_or("missing accepted timing boundary")?;
        let current = (boundary.seconds, boundary.nanos);
        assert!(
            current >= previous,
            "task timing boundaries changed causal order"
        );
        previous = current;
    }
    assert!(timing.publication_authorized_at.is_none());
    assert!(timing.publication_result_accepted_at.is_none());
    assert!(matches!(
        store
            .get_task_timing(
                wrong,
                &preparation.task_id,
                &preparation.attempt_id,
                max_seconds
            )
            .await,
        Err(StoreError::NotFound)
    ));
    let replay = store
        .get_task_timing(
            scope,
            &preparation.task_id,
            &preparation.attempt_id,
            max_seconds,
        )
        .await?;
    assert_eq!(timing.queue_accepted_at, replay.queue_accepted_at);
    assert_eq!(timing.claim_accepted_at, replay.claim_accepted_at);
    assert_eq!(
        timing.coding_result_accepted_at,
        replay.coding_result_accepted_at
    );
    assert_eq!(
        timing.quality_result_accepted_at,
        replay.quality_result_accepted_at
    );
    assert_eq!(timing.execution_stopped_at, replay.execution_stopped_at);
    Ok(())
}
