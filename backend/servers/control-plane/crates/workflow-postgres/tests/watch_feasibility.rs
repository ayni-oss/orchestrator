use ayni_orchestrator_workflow_postgres::watch::{
    WatchService, append_update, append_update_timed, reset_conversation,
};
use ayni_orchestrator_workflow_postgres::watch_v1::workflow_watch_feasibility_client::WorkflowWatchFeasibilityClient;
use ayni_orchestrator_workflow_postgres::watch_v1::workflow_watch_feasibility_server::WorkflowWatchFeasibilityServer;
use ayni_orchestrator_workflow_postgres::watch_v1::{
    GetSnapshotRequest, WatchConversationRequest, WatchFrame, watch_frame,
};
use ayni_orchestrator_workflow_postgres::{connect, migrate};
use std::error::Error;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Code, Status, Streaming, transport::Endpoint, transport::Server};

const DATABASE_URL_ENV: &str = "AYNI_POSTGRES_FEASIBILITY_URL";
const CONVERSATION: &str = "conversation:018f0f7b-6a4d-7a11-8d77-414243444546";
const OTHER_CONVERSATION: &str = "conversation:018f0f7b-6a4d-7a11-8d77-515253545556";
type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;
type WatchClient = WorkflowWatchFeasibilityClient<tonic::transport::Channel>;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a disposable PostgreSQL 17 instance"]
async fn tonic_snapshot_watch_feasibility_gate() -> TestResult {
    let database_url = std::env::var(DATABASE_URL_ENV)
        .map_err(|_| format!("set {DATABASE_URL_ENV} to a disposable PostgreSQL database"))?;
    migrate(&database_url).await?;
    reset_conversation(&database_url, CONVERSATION, "Feasibility conversation").await?;
    reset_conversation(&database_url, OTHER_CONVERSATION, "Other conversation").await?;

    let service = WatchService::new(database_url.clone()).with_limits(
        128,
        Duration::from_millis(10),
        Duration::from_secs(60),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(
        Server::builder()
            .add_service(WorkflowWatchFeasibilityServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    let mut client = WorkflowWatchFeasibilityClient::connect(format!("http://{address}")).await?;

    prove_atomic_snapshot_boundary(&database_url, &mut client).await?;
    let snapshot_latencies = measure_snapshot_race(&database_url, &mut client).await?;
    prove_cursorless_watch(&database_url, &mut client).await?;
    prove_notification_replay_and_expiry(&database_url, &mut client).await?;
    prove_slow_reader_recovery(&database_url, &mut client).await?;
    let watch_latencies = measure_watch_latency(&database_url, &mut client).await?;
    print_latency_evidence(&snapshot_latencies, &watch_latencies);

    server.abort();
    Ok(())
}

async fn prove_atomic_snapshot_boundary(
    database_url: &str,
    client: &mut WatchClient,
) -> TestResult {
    let (mut pending_client, pending_driver) = connect(database_url).await?;
    let pending = pending_client.transaction().await?;
    let pending_revision: i64 = pending
        .query_one(
            "UPDATE workflow_watch_conversations SET revision = revision + 1 \
             WHERE id = $1 RETURNING revision",
            &[&CONVERSATION],
        )
        .await?
        .get(0);
    let pending_cursor: i64 = pending
        .query_one(
            "INSERT INTO workflow_watch_updates \
             (conversation_id, revision, kind, safe_payload) VALUES ($1, $2, 'boundary', 'held') \
             RETURNING cursor",
            &[&CONVERSATION, &pending_revision],
        )
        .await?
        .get(0);
    pending
        .query_one(
            "SELECT pg_notify('workflow_watch_update', $1)",
            &[&CONVERSATION],
        )
        .await?;
    let mut watch = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: None,
        })
        .await?
        .into_inner();
    let snapshot_frame = next_frame(
        &mut watch,
        "atomic-boundary watch ended before its snapshot",
    )
    .await?;
    let Some(watch_frame::Frame::Snapshot(snapshot)) = snapshot_frame.frame else {
        return Err("atomic-boundary watch did not begin with a snapshot".into());
    };
    assert_eq!(snapshot.revision, 0);
    assert_eq!(cursor_value(&snapshot.resume_cursor)?, 0);
    pending.commit().await?;
    let update_frame = next_frame(
        &mut watch,
        "atomic-boundary watch lost the concurrent update",
    )
    .await?;
    assert_eq!(frame_cursor(&update_frame)?, pending_cursor);
    let Some(watch_frame::Frame::Update(update)) = update_frame.frame else {
        return Err("atomic-boundary watch emitted a non-update frame".into());
    };
    assert_eq!(update.revision, 1);
    pending_driver.abort();
    reset_conversation(database_url, CONVERSATION, "Feasibility conversation").await?;
    Ok(())
}

async fn measure_snapshot_race(
    database_url: &str,
    client: &mut WatchClient,
) -> TestResult<Vec<Duration>> {
    let update_url = database_url.to_owned();
    let updater = tokio::spawn(async move {
        for index in 0..100_u64 {
            append_update(
                &update_url,
                CONVERSATION,
                "race",
                &format!("update-{index}"),
                true,
            )
            .await?;
            sleep(Duration::from_millis(1)).await;
        }
        Ok::<_, tokio_postgres::Error>(())
    });
    let mut latencies = Vec::with_capacity(110);
    for _ in 0..110 {
        let started = Instant::now();
        let snapshot = client
            .get_snapshot(GetSnapshotRequest {
                conversation_id: CONVERSATION.into(),
            })
            .await?
            .into_inner();
        latencies.push(started.elapsed());
        let boundary = cursor_value(&snapshot.resume_cursor)?;
        assert_eq!(
            snapshot.revision,
            revision_at_boundary(database_url, boundary).await?
        );
    }
    updater.await??;
    Ok(latencies)
}

async fn prove_cursorless_watch(database_url: &str, client: &mut WatchClient) -> TestResult {
    let mut watch = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: None,
        })
        .await?
        .into_inner();
    let snapshot_frame =
        next_frame(&mut watch, "cursorless watch ended before its snapshot").await?;
    let Some(watch_frame::Frame::Snapshot(snapshot)) = snapshot_frame.frame else {
        return Err("cursorless watch did not begin with a snapshot".into());
    };
    let snapshot_cursor = cursor_value(&snapshot.resume_cursor)?;
    let (next_cursor, _) = append_update(
        database_url,
        CONVERSATION,
        "after_snapshot",
        "visible",
        true,
    )
    .await?;
    let update_frame = next_frame(&mut watch, "watch ended before its update").await?;
    let Some(watch_frame::Frame::Update(update)) = update_frame.frame else {
        return Err("watch returned a non-update frame".into());
    };
    assert!(cursor_value(&update.cursor)? > snapshot_cursor);
    assert_eq!(cursor_value(&update.cursor)?, next_cursor);
    Ok(())
}

async fn prove_notification_replay_and_expiry(
    database_url: &str,
    client: &mut WatchClient,
) -> TestResult {
    let latest = client
        .get_snapshot(GetSnapshotRequest {
            conversation_id: CONVERSATION.into(),
        })
        .await?
        .into_inner();
    let mut watch = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: Some(latest.resume_cursor),
        })
        .await?
        .into_inner();
    let (lost_cursor, _) = append_update(
        database_url,
        CONVERSATION,
        "lost_notification",
        "replayed",
        false,
    )
    .await?;
    let replayed = timeout(Duration::from_millis(500), watch.message())
        .await??
        .ok_or("lost notification was not recovered from the table")?;
    let Some(watch_frame::Frame::Update(replayed)) = replayed.frame else {
        return Err("lost-notification replay was not an update".into());
    };
    assert_eq!(cursor_value(&replayed.cursor)?, lost_cursor);
    reject_cross_conversation_cursor(client, &replayed.cursor).await?;
    let pruned_cursor = advance_retention_watermark(database_url).await?;
    assert!(pruned_cursor > lost_cursor);
    assert_active_watch_expired(&mut watch).await?;
    assert_expired_cursor_rejected(client, lost_cursor - 1).await?;
    assert_cursorless_recovery(client).await?;
    Ok(())
}

async fn reject_cross_conversation_cursor(client: &mut WatchClient, cursor: &str) -> TestResult {
    let Err(status) = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: OTHER_CONVERSATION.into(),
            after_cursor: Some(cursor.to_owned()),
        })
        .await
    else {
        return Err("cross-conversation cursor unexpectedly succeeded".into());
    };
    assert_eq!(status.code(), Code::InvalidArgument);
    Ok(())
}

async fn advance_retention_watermark(database_url: &str) -> TestResult<i64> {
    let (mut client, driver) = connect(database_url).await?;
    let transaction = client.transaction().await?;
    let revision: i64 = transaction
        .query_one(
            "UPDATE workflow_watch_conversations SET revision = revision + 1 \
             WHERE id = $1 RETURNING revision",
            &[&CONVERSATION],
        )
        .await?
        .get(0);
    let cursor: i64 = transaction
        .query_one(
            "INSERT INTO workflow_watch_updates \
             (conversation_id, revision, kind, safe_payload) \
             VALUES ($1, $2, 'retention_race', 'pruned-before-replay') RETURNING cursor",
            &[&CONVERSATION, &revision],
        )
        .await?
        .get(0);
    transaction
        .execute(
            "DELETE FROM workflow_watch_updates WHERE conversation_id = $1 AND cursor <= $2",
            &[&CONVERSATION, &cursor],
        )
        .await?;
    transaction
        .execute(
            "UPDATE workflow_watch_watermarks SET pruned_through_cursor = $2 \
             WHERE conversation_id = $1",
            &[&CONVERSATION, &cursor],
        )
        .await?;
    transaction.commit().await?;
    driver.abort();
    Ok(cursor)
}

async fn assert_active_watch_expired(watch: &mut Streaming<WatchFrame>) -> TestResult {
    let Err(status) = timeout(Duration::from_secs(2), watch.message()).await? else {
        return Err("active watch silently crossed its retained watermark".into());
    };
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert_eq!(status.message(), "WATCH_CURSOR_EXPIRED");
    Ok(())
}

async fn assert_expired_cursor_rejected(client: &mut WatchClient, cursor: i64) -> TestResult {
    let Err(status) = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: Some(format!("v1|{CONVERSATION}|{cursor}")),
        })
        .await
    else {
        return Err("expired cursor unexpectedly succeeded".into());
    };
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert_eq!(status.message(), "WATCH_CURSOR_EXPIRED");
    Ok(())
}

async fn assert_cursorless_recovery(client: &mut WatchClient) -> TestResult {
    let frame = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: None,
        })
        .await?
        .into_inner()
        .message()
        .await?
        .ok_or("snapshot recovery watch ended")?;
    assert!(matches!(frame.frame, Some(watch_frame::Frame::Snapshot(_))));
    Ok(())
}

async fn prove_slow_reader_recovery(database_url: &str, client: &mut WatchClient) -> TestResult {
    let slow_service = WatchService::new(database_url.to_owned()).with_limits(
        1,
        Duration::from_millis(5),
        Duration::from_secs(60),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(
        Server::builder()
            .add_service(WorkflowWatchFeasibilityServer::new(slow_service))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    let channel = Endpoint::from_shared(format!("http://{address}"))?
        .initial_stream_window_size(1_024)
        .connect()
        .await?;
    let mut slow_client = WorkflowWatchFeasibilityClient::new(channel);
    let resume_cursor = client
        .get_snapshot(GetSnapshotRequest {
            conversation_id: CONVERSATION.into(),
        })
        .await?
        .into_inner()
        .resume_cursor;
    let mut healthy = open_watch(client, resume_cursor.clone()).await?;
    let mut slow = open_watch(&mut slow_client, resume_cursor.clone()).await?;
    let committed = append_backpressure_updates(database_url).await?;
    sleep(Duration::from_millis(100)).await;
    let initial_slow_cursor = cursor_value(&resume_cursor)?;
    let (eviction, last_slow_cursor) = observe_eviction(&mut slow, initial_slow_cursor).await?;
    assert_eq!(eviction.code(), Code::ResourceExhausted);
    assert_stream_cursors(&mut healthy, &committed).await?;
    assert!(last_slow_cursor < *committed.last().ok_or("no backpressure updates")?);
    assert_resumed_cursors(client, &committed, last_slow_cursor).await?;
    server.abort();
    Ok(())
}

async fn open_watch(
    client: &mut WatchClient,
    after_cursor: String,
) -> TestResult<Streaming<WatchFrame>> {
    Ok(client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: Some(after_cursor),
        })
        .await?
        .into_inner())
}

async fn append_backpressure_updates(database_url: &str) -> TestResult<Vec<i64>> {
    let payload = "x".repeat(8 * 1_024);
    let mut cursors = Vec::with_capacity(32);
    for index in 0..32 {
        let (cursor, _) = append_update(
            database_url,
            CONVERSATION,
            "backpressure",
            &format!("queued-{index}:{payload}"),
            true,
        )
        .await?;
        cursors.push(cursor);
    }
    Ok(cursors)
}

async fn observe_eviction(
    stream: &mut Streaming<WatchFrame>,
    mut last_cursor: i64,
) -> TestResult<(Status, i64)> {
    loop {
        match timeout(Duration::from_secs(2), stream.message()).await? {
            Ok(Some(frame)) => {
                let cursor = frame_cursor(&frame)?;
                assert!(cursor > last_cursor);
                last_cursor = cursor;
            }
            Err(status) => return Ok((status, last_cursor)),
            Ok(None) => return Err("slow watcher ended without a status".into()),
        }
    }
}

async fn assert_stream_cursors(stream: &mut Streaming<WatchFrame>, expected: &[i64]) -> TestResult {
    for expected_cursor in expected {
        let frame = next_frame(stream, "healthy watcher ended during slow-reader test").await?;
        assert_eq!(frame_cursor(&frame)?, *expected_cursor);
    }
    Ok(())
}

async fn assert_resumed_cursors(
    client: &mut WatchClient,
    expected: &[i64],
    after: i64,
) -> TestResult {
    let mut stream = open_watch(client, format!("v1|{CONVERSATION}|{after}")).await?;
    for expected_cursor in expected.iter().copied().filter(|cursor| *cursor > after) {
        let frame = next_frame(&mut stream, "resumed slow watcher ended during replay").await?;
        assert_eq!(frame_cursor(&frame)?, expected_cursor);
    }
    Ok(())
}

async fn measure_watch_latency(
    database_url: &str,
    client: &mut WatchClient,
) -> TestResult<Vec<Duration>> {
    let current = client
        .get_snapshot(GetSnapshotRequest {
            conversation_id: CONVERSATION.into(),
        })
        .await?
        .into_inner();
    let mut watch = open_watch(client, current.resume_cursor).await?;
    let mut latencies = Vec::with_capacity(110);
    for index in 0..110_u64 {
        let (expected_cursor, _, started) = append_update_timed(
            database_url,
            CONVERSATION,
            "latency",
            &format!("sample-{index}"),
            true,
        )
        .await?;
        let frame = next_frame(&mut watch, "latency watch ended").await?;
        assert_eq!(frame_cursor(&frame)?, expected_cursor);
        latencies.push(started.elapsed());
    }
    Ok(latencies)
}

async fn next_frame(stream: &mut Streaming<WatchFrame>, ended: &str) -> TestResult<WatchFrame> {
    timeout(Duration::from_secs(2), stream.message())
        .await??
        .ok_or_else(|| ended.into())
}

fn print_latency_evidence(snapshot_latencies: &[Duration], watch_latencies: &[Duration]) {
    let (snapshot_p50, snapshot_p95, snapshot_max) = distribution(&snapshot_latencies[10..]);
    let (watch_p50, watch_p95, watch_max) = distribution(&watch_latencies[10..]);
    println!(
        "tonic-watch-feasibility: atomic-boundary=pass notification-replay=pass cursor-expiry=pass slow-reader=pass snapshot-p50-ms={:.3} snapshot-p95-ms={:.3} snapshot-max-ms={:.3} commit-ack-to-client-watch-p50-ms={:.3} commit-ack-to-client-watch-p95-ms={:.3} commit-ack-to-client-watch-max-ms={:.3} samples=100",
        snapshot_p50.as_secs_f64() * 1000.0,
        snapshot_p95.as_secs_f64() * 1000.0,
        snapshot_max.as_secs_f64() * 1000.0,
        watch_p50.as_secs_f64() * 1000.0,
        watch_p95.as_secs_f64() * 1000.0,
        watch_max.as_secs_f64() * 1000.0,
    );
}

async fn revision_at_boundary(database_url: &str, boundary: i64) -> TestResult<u64> {
    let (client, driver) = connect(database_url).await?;
    let revision: i64 = client
        .query_one(
            "SELECT COALESCE(max(revision), 0) FROM workflow_watch_updates \
             WHERE conversation_id = $1 AND cursor <= $2",
            &[&CONVERSATION, &boundary],
        )
        .await?
        .get(0);
    driver.abort();
    Ok(u64::try_from(revision)?)
}

fn frame_cursor(frame: &WatchFrame) -> TestResult<i64> {
    match &frame.frame {
        Some(watch_frame::Frame::Update(update)) => cursor_value(&update.cursor),
        _ => Err("expected an update frame".into()),
    }
}

fn cursor_value(cursor: &str) -> TestResult<i64> {
    cursor
        .rsplit('|')
        .next()
        .ok_or_else(|| Box::<dyn Error + Send + Sync>::from("cursor has no value"))?
        .parse()
        .map_err(Into::into)
}

fn distribution(values: &[Duration]) -> (Duration, Duration, Duration) {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    (sorted[49], sorted[94], sorted[99])
}
