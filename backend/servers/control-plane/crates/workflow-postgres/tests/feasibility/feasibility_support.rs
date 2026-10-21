use super::*;

pub(super) async fn prove_capacity_race(
    client: &Client,
    store: &WorkflowStore,
    workspace_id: &str,
    project_id: &str,
    conversation_id: &str,
) -> TestResult {
    for index in 17..96 {
        store
            .send_message(
                &format!("capacity-{index}"),
                workspace_id,
                project_id,
                conversation_id,
                "queued",
            )
            .await?;
    }
    let callers = 8;
    let barrier = Arc::new(Barrier::new(callers));
    let mut sends = Vec::new();
    for index in 0..callers {
        let store = store.clone();
        let barrier = Arc::clone(&barrier);
        let workspace_id = workspace_id.to_owned();
        let project_id = project_id.to_owned();
        let conversation_id = conversation_id.to_owned();
        sends.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .send_message(
                    &format!("capacity-race-{index}"),
                    &workspace_id,
                    &project_id,
                    &conversation_id,
                    "queued",
                )
                .await
        }));
    }
    let mut admitted = 0;
    let mut rejected = 0;
    for send in sends {
        match send.await? {
            Ok(_) => admitted += 1,
            Err(StoreError::Capacity("turn_queue")) => rejected += 1,
            Err(error) => return Err(Box::new(error)),
        }
    }
    assert_eq!(admitted, 4);
    assert_eq!(rejected, 4);
    let retained: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_turns WHERE conversation_id = $1",
            &[&conversation_id],
        )
        .await?
        .get(0);
    assert_eq!(retained, 100);
    Ok(())
}

pub(super) async fn prove_future_version_rejected(
    client: &Client,
    database_url: &str,
) -> TestResult {
    client.execute("INSERT INTO workflow_schema_migrations (version, description, checksum) VALUES (3, 'future', 'sha256:future')", &[]).await?;
    assert!(matches!(
        validate(database_url).await,
        Err(MigrationError::FutureVersion(3))
    ));
    client
        .execute(
            "DELETE FROM workflow_schema_migrations WHERE version = 3",
            &[],
        )
        .await?;
    Ok(())
}
