use ayni_orchestrator_telemetry::v1::{LogSource, ReadLogsRequest, TelemetryRecord};
use ayni_orchestrator_workflow_postgres::{
    WorkflowStore, migrate_for_runtime, validate_for_runtime,
};

#[tokio::test]
#[ignore = "requires disposable PostgreSQL with runtime role"]
async fn log_replay_is_durable_scoped_and_idempotent()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let owner = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let runtime = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    let role = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?;
    migrate_for_runtime(&owner, &role).await?;
    validate_for_runtime(&runtime, &role).await?;
    let store = WorkflowStore::new(runtime.clone());
    let source = LogSource {
        id: uuid::Uuid::new_v4().to_string(),
        execution_id: uuid::Uuid::new_v4().to_string(),
        pod_uid: uuid::Uuid::new_v4().to_string(),
        ..Default::default()
    };
    let mut record = TelemetryRecord {
        source: Some(source.clone()),
        sequence: 1,
        event: "output".into(),
        stream: "stderr".into(),
        data: vec![0, 255, 10],
        ..Default::default()
    };
    store.append_log(&record).await?;
    store.append_log(&record).await?;
    record.data = vec![1];
    assert!(store.append_log(&record).await.is_err());
    record.sequence = 3;
    assert!(store.append_log(&record).await.is_err());
    record.sequence = 2;
    record.data = vec![0, 255, 10];
    store.append_log(&record).await?;
    let reopened = WorkflowStore::new(runtime);
    let page = reopened
        .read_logs(&ReadLogsRequest {
            source_id: source.id.clone(),
            ..Default::default()
        })
        .await?;
    assert_eq!(page.entries.len(), 2);
    assert_eq!(
        page.entries[0].record.as_ref().map(|r| &r.data),
        Some(&vec![0, 255, 10])
    );
    assert_eq!(
        reopened.log_sources(&source.execution_id, "").await?.len(),
        1
    );
    assert!(reopened.log_sources("unrelated", "").await?.is_empty());
    reopened.cleanup_logs(86400, 1).await?;
    let expired = reopened
        .read_logs(&ReadLogsRequest {
            source_id: source.id,
            ..Default::default()
        })
        .await?;
    assert!(expired.entries.is_empty());
    assert!(expired.expired_through_cursor >= page.next_cursor);
    // Lost acknowledgements remain replayable after the raw payload has expired.
    record.sequence = 1;
    reopened.append_log(&record).await?;
    record.data = vec![1];
    assert!(reopened.append_log(&record).await.is_err());
    let metrics = reopened.log_metrics().await?;
    assert!(metrics.contains("workflow_capture_retained_payload_bytes 0"));
    Ok(())
}
