//! Retained history uses production admission, runtime grants and replay cleanup.
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, connect, migrate_for_runtime, validate,
};
use std::error::Error;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL acceptance fixture"]
async fn timeline_survives_cleanup_restart_and_exact_command_replay() -> Result<(), Box<dyn Error>>
{
    let admin_url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let runtime_url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    let role = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?;
    migrate_for_runtime(&admin_url, &role).await?;
    let store = WorkflowStore::new(runtime_url.clone());
    let key = Uuid::now_v7().to_string();
    let workspace = store.create_workspace(&key, "Timeline").await?.resource;
    let project = store
        .create_project(&key, &workspace.id, "Timeline", "")
        .await?
        .resource;
    let conversation = store
        .create_conversation(&key, &workspace.id, &project.id, "Timeline")
        .await?
        .resource;
    let (w, p, c) = (&workspace.id[..], &project.id[..], &conversation.id[..]);
    store.send_message("first", w, p, c, "First").await?;
    store.send_message("second", w, p, c, "Second").await?;
    let first = store.list_execution_timeline(w, p, c, 0, 0, 1).await?;
    assert_eq!(first.entries.len(), 1);
    assert_ne!(first.next_before_revision, 0);
    assert!(!first.entries[0].activity.turn_id.is_empty());
    assert!(first.entries[0].activity.cursor.is_empty());
    let boundary = first.snapshot_revision;
    store.send_message("third", w, p, c, "Third").await?;
    store.send_message("third", w, p, c, "Third").await?;
    let older = store
        .list_execution_timeline(w, p, c, boundary, first.next_before_revision, 100)
        .await?;
    assert!(
        older
            .entries
            .iter()
            .all(|entry| entry.activity.conversation_revision < first.next_before_revision)
    );
    assert_eq!(older.snapshot_revision, boundary);
    let before = store.list_execution_timeline(w, p, c, 0, 0, 100).await?;
    assert_eq!(
        before.snapshot_revision,
        boundary + 1,
        "exact retry duplicated progress"
    );
    let (client, driver) = connect(&admin_url).await?;
    client.execute("UPDATE workflow_updates SET accepted_at = clock_timestamp() - interval '8 days' WHERE conversation_id = $1", &[&c]).await?;
    assert!(store.cleanup_observation().await? > 0);
    let restarted = WorkflowStore::new(runtime_url.clone());
    let after = restarted
        .list_execution_timeline(w, p, c, before.snapshot_revision, 0, 100)
        .await?;
    assert_eq!(before.entries, after.entries);
    let remaining: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_updates WHERE conversation_id = $1",
            &[&c],
        )
        .await?
        .get(0);
    assert_eq!(
        remaining, 0,
        "target conversation still retained watch deltas"
    );
    assert_eq!(before.retained_from_revision, after.retained_from_revision);
    assert!(matches!(
        restarted
            .list_execution_timeline(w, "foreign-project", c, 0, 0, 10)
            .await,
        Err(StoreError::NotFound)
    ));
    for (snapshot, before, limit) in [
        (0, 1, 10),
        (u64::MAX, 0, 10),
        (boundary, boundary + 1, 10),
        (0, 0, 101),
    ] {
        assert!(matches!(
            restarted
                .list_execution_timeline(w, p, c, snapshot, before, limit)
                .await,
            Err(StoreError::InvalidInput(_))
        ));
    }
    prove_retention_guards(&client, &runtime_url, c).await?;
    driver.abort();
    Ok(())
}

async fn prove_retention_guards(
    client: &tokio_postgres::Client,
    runtime_url: &str,
    conversation: &str,
) -> Result<(), Box<dyn Error>> {
    let (runtime, runtime_driver) = connect(runtime_url).await?;
    assert!(
        runtime
            .execute(
                "DELETE FROM workflow_execution_timeline WHERE conversation_id = $1",
                &[&conversation]
            )
            .await
            .is_err()
    );
    client
        .batch_execute(
            "ALTER TABLE workflow_updates DISABLE TRIGGER workflow_retain_execution_activity",
        )
        .await?;
    let rejected = validate(runtime_url).await.is_err();
    client
        .batch_execute(
            "ALTER TABLE workflow_updates ENABLE TRIGGER workflow_retain_execution_activity",
        )
        .await?;
    assert!(
        rejected,
        "disabled retention trigger passed runtime readiness"
    );
    validate(runtime_url).await?;
    runtime_driver.abort();
    Ok(())
}
