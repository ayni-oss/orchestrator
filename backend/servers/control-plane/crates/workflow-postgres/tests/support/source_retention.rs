use super::{Fixture, Result, connect, key as new_key};
use ayni_orchestrator_workflow_postgres::{WorkflowStore, store::task_recovery::RecoveryInput};

pub async fn prove(f: &Fixture, attempt: &str, key: &str, patch: &str, quality: &str) -> Result {
    let before = f
        .store
        .get_task_run(f.scope(), &f.attempts[0].task_id, attempt)
        .await?;
    let (admin, driver) = connect(&f.admin).await?;
    f.store.cleanup_retained_task_sources().await?;
    let row=admin.query_one("SELECT octet_length(source_pack),source_reclaim_after>clock_timestamp()+interval '29 days' FROM workflow_task_runs WHERE attempt_id=$1",&[&attempt]).await?;
    assert!(row.get::<_, i32>(0) > 0);
    assert!(row.get::<_, bool>(1));
    // An unsettled provider blocks reclamation even if the draft is retained.
    admin.execute("UPDATE workflow_task_runs SET provider_state='uncertain',source_reclaim_after=clock_timestamp()-interval '1 second' WHERE attempt_id=$1",&[&attempt]).await?;
    f.store.cleanup_retained_task_sources().await?;
    let row=admin.query_one("SELECT octet_length(source_pack),source_reclaim_after IS NULL FROM workflow_task_runs WHERE attempt_id=$1",&[&attempt]).await?;
    assert!(row.get::<_, i32>(0) > 0);
    assert!(row.get::<_, bool>(1));
    admin
        .execute(
            "UPDATE workflow_task_runs SET provider_state='settled' WHERE attempt_id=$1",
            &[&attempt],
        )
        .await?;
    question_resets_grace(f, &admin, attempt).await?;
    f.store.cleanup_retained_task_sources().await?;
    // Process replacement preserves the same eligibility timestamp, rather than
    // extending every eligible archive's deadline at each maintenance sweep.
    let reopening = WorkflowStore::new(f.runtime.clone());
    let scheduled: String = admin
        .query_one(
            "SELECT source_reclaim_after::text FROM workflow_task_runs WHERE attempt_id=$1",
            &[&attempt],
        )
        .await?
        .get(0);
    reopening.cleanup_retained_task_sources().await?;
    let unchanged: String = admin
        .query_one(
            "SELECT source_reclaim_after::text FROM workflow_task_runs WHERE attempt_id=$1",
            &[&attempt],
        )
        .await?
        .get(0);
    assert_eq!(scheduled, unchanged);
    admin.execute("UPDATE workflow_task_runs SET source_reclaim_after=clock_timestamp()-interval '1 second' WHERE attempt_id=$1",&[&attempt]).await?;
    reopening.cleanup_retained_task_sources().await?;
    let row=admin.query_one("SELECT octet_length(source_pack),source_retired_at IS NOT NULL,source_retired_bytes FROM workflow_task_runs WHERE attempt_id=$1",&[&attempt]).await?;
    assert_eq!(row.get::<_, i32>(0), 0);
    assert!(row.get::<_, bool>(1));
    assert!(row.get::<_, i64>(2) > 0);
    // Evidence and exact publication receipt replay survive reclamation/restart.
    assert_eq!(
        before,
        f.store
            .get_task_run(f.scope(), &f.attempts[0].task_id, attempt)
            .await?
    );
    let reopened = ayni_orchestrator_workflow_postgres::WorkflowStore::new(f.runtime.clone());
    let receipt = reopened
        .start_task_publication(
            f.scope(),
            &f.attempts[0].task_id,
            attempt,
            key,
            patch,
            quality,
        )
        .await?;
    assert_eq!(receipt.publication_id, attempt);
    let metrics = reopened.operational_metrics().await?;
    assert!(metrics.contains("workflow_task_source_bytes_reclaimed "));
    // A source whose only outcome is failed remains retained for recovery.
    let retained=admin.query_one("SELECT bool_and(octet_length(source_pack)>0) FROM workflow_task_runs WHERE attempt_id<>$1 AND source_reclaim_after IS NULL",&[&attempt]).await?;
    assert_ne!(retained.get::<_, Option<bool>>(0), Some(false));
    assert_eq!(reopened.cleanup_retained_task_sources().await?, 0);
    driver.abort();
    Ok(())
}

// Review decisions have their own authority: a retained draft alone cannot retire
// source while an unanswered question may still require the original checkout.
async fn question_resets_grace(
    f: &Fixture,
    admin: &tokio_postgres::Client,
    attempt: &str,
) -> Result {
    let task = &f.attempts[0].task_id;
    f.store.cleanup_retained_task_sources().await?;
    admin.execute("UPDATE workflow_task_runs SET source_reclaim_after=clock_timestamp()-interval '1 second' WHERE attempt_id=$1", &[&attempt]).await?;
    let question = f
        .store
        .record_task_recovery(
            f.scope(),
            task,
            attempt,
            &new_key(),
            &RecoveryInput::Question {
                text: "Retain the exact source until this review decision is answered.".into(),
            },
        )
        .await?;
    let reopened = WorkflowStore::new(f.runtime.clone());
    assert_eq!(reopened.cleanup_retained_task_sources().await?, 0);
    let row = admin.query_one("SELECT octet_length(source_pack)>0,source_reclaim_after IS NULL FROM workflow_task_runs WHERE attempt_id=$1", &[&attempt]).await?;
    assert!(
        row.get::<_, bool>(0),
        "unanswered review must protect the source"
    );
    assert!(
        row.get::<_, bool>(1),
        "lost eligibility must reset the grace period"
    );
    // Reopening with the question still unresolved cannot silently restore eligibility.
    WorkflowStore::new(f.runtime.clone())
        .cleanup_retained_task_sources()
        .await?;
    let row = admin
        .query_one(
            "SELECT source_reclaim_after IS NULL FROM workflow_task_runs WHERE attempt_id=$1",
            &[&attempt],
        )
        .await?;
    assert!(row.get::<_, bool>(0));
    reopened.record_task_recovery(
        f.scope(), task, attempt, &new_key(),
        &RecoveryInput::Answer { question_id: question.id, text: "The retained draft is sufficient; release the archive after a new grace period.".into() },
    ).await?;
    reopened.cleanup_retained_task_sources().await?;
    let row = admin.query_one("SELECT octet_length(source_pack)>0,source_reclaim_after>clock_timestamp()+interval '29 days' FROM workflow_task_runs WHERE attempt_id=$1", &[&attempt]).await?;
    assert!(row.get::<_, bool>(0));
    assert!(
        row.get::<_, bool>(1),
        "resolved review must receive a fresh full grace period"
    );
    Ok(())
}
