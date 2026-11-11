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

#[tokio::test]
#[ignore = "requires disposable PostgreSQL with runtime role"]
async fn log_connections_reuse_independent_reader_and_writer_and_recover()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let owner = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let runtime = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    let role = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?;
    migrate_for_runtime(&owner, &role).await?;
    let application = format!("log-writer-{}", uuid::Uuid::new_v4());
    let separator = if runtime.contains('?') { '&' } else { '?' };
    let store = WorkflowStore::new(format!(
        "{runtime}{separator}application_name={application}"
    ));
    let (observer, _driver) = ayni_orchestrator_workflow_postgres::connect(&owner).await?;
    let mut record = TelemetryRecord {
        source: Some(LogSource {
            id: uuid::Uuid::new_v4().to_string(),
            ..Default::default()
        }),
        sequence: 1,
        event: "output".into(),
        data: b"same\n".to_vec(),
        ..Default::default()
    };
    store.append_log(&record).await?;
    let first: i32 = observer
        .query_one(
            "SELECT pid FROM pg_stat_activity WHERE application_name=$1",
            &[&application],
        )
        .await?
        .get(0);
    record.sequence = 2;
    store.clone().append_log(&record).await?;
    let reused: i32 = observer
        .query_one(
            "SELECT pid FROM pg_stat_activity WHERE application_name=$1",
            &[&application],
        )
        .await?
        .get(0);
    assert_eq!(
        first, reused,
        "cloned stores must reuse the dedicated log writer"
    );
    observer
        .execute("SELECT pg_terminate_backend($1)", &[&first])
        .await?;
    // The first write can race the disconnect notification. Retry the same exact
    // record, just as telemetry does when an acknowledgement is lost.
    record.sequence = 3;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if store.append_log(&record).await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await?;
    store.append_log(&record).await?;
    assert_reader_recovery(&store, &observer, &application, &record).await
}

async fn assert_reader_recovery(
    store: &WorkflowStore,
    observer: &tokio_postgres::Client,
    application: &str,
    record: &TelemetryRecord,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let query = ReadLogsRequest {
        source_id: record.source.as_ref().ok_or("source missing")?.id.clone(),
        ..Default::default()
    };
    let writer: i32 = observer
        .query_one(
            "SELECT pid FROM pg_stat_activity WHERE application_name=$1",
            &[&application],
        )
        .await?
        .get(0);
    let page = store.read_logs(&query).await?;
    assert_eq!(page.entries.len(), 3);
    assert!(
        page.entries
            .iter()
            .all(|entry| entry.record.as_ref().is_some_and(|r| r.data == b"same\n"))
    );
    let reader: i32 = observer
        .query_one(
            "SELECT pid FROM pg_stat_activity WHERE application_name=$1 AND pid<>$2",
            &[&application, &writer],
        )
        .await?
        .get(0);
    store.clone().log_sources("", "").await?;
    store.clone().read_logs(&query).await?;
    let reused_reader: i32 = observer
        .query_one(
            "SELECT pid FROM pg_stat_activity WHERE application_name=$1 AND pid<>$2",
            &[&application, &writer],
        )
        .await?
        .get(0);
    assert_eq!(
        reader, reused_reader,
        "inventory and history share their own connection"
    );
    observer
        .execute("SELECT pg_terminate_backend($1)", &[&reader])
        .await?;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(recovered) = store.read_logs(&query).await {
                assert_eq!(recovered, page);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await?;
    Ok(())
}
