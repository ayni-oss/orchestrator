use super::*;

#[test]
fn lifecycle_approval_rolls_back_task_receipt_and_archive_when_compaction_fails()
-> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("send-lifecycle", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    let mut c =
        store.decide_project_plan("approve-before-cancel", &plan_decision(&c, "approve")?)?;
    let task = store.development_task(&c.proposals[0].task_id)?;
    let attempt = store.development_attempt(&c.proposals[0].attempt_id)?;
    let mut cancel = c.proposals[0].clone();
    cancel.id = "cancel-context-test".into();
    cancel.action = "cancel_task".into();
    cancel.target_task_id = task.id.clone();
    cancel.state = "pending".into();
    c.proposals.push(cancel);
    store.save_conversation(&mut c)?;
    let before = c.clone();
    let context = store.project_conversation_context(&c.project_id, &c.id)?;
    store.connection.execute_batch("CREATE TRIGGER reject_lifecycle_compaction BEFORE UPDATE ON project_context_summaries BEGIN SELECT RAISE(ABORT,'interrupted compaction'); END;").map_err(CleanStoreError::Database)?;
    let decision = v1::DecideProjectProposalRequest {
        idempotency_key: "cancel-context".into(),
        conversation_id: c.id.clone(),
        proposal_id: "cancel-context-test".into(),
        decision: "approve".into(),
    };
    assert!(
        store
            .decide_project_proposal("cancel-context", &decision)
            .is_err()
    );
    assert_eq!(store.development_task(&task.id)?, task);
    assert_eq!(store.development_attempt(&attempt.id)?, attempt);
    assert_eq!(store.project_conversation(&c.id)?, before);
    assert_eq!(
        store.project_conversation_context(&c.project_id, &c.id)?,
        context
    );
    let count: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM durable_actions WHERE operation='CancelDevelopmentTask'",
            [],
            |r| r.get(0),
        )
        .map_err(CleanStoreError::Database)?;
    assert_eq!(count, 0);
    assert!(
        store
            .replay_conversation("cancel-context", &decision)?
            .is_none()
    );
    store
        .connection
        .execute_batch("DROP TRIGGER reject_lifecycle_compaction")
        .map_err(CleanStoreError::Database)?;
    store.decide_project_proposal("cancel-context", &decision)?;
    assert_eq!(
        store.development_task(&task.id)?.state,
        v1::DevelopmentTaskState::Cancelled as i32
    );
    Ok(())
}
