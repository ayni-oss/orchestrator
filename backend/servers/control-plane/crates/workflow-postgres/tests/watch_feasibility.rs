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
use tonic::{Code, transport::Endpoint, transport::Server};

const DATABASE_URL_ENV: &str = "AYNI_POSTGRES_FEASIBILITY_URL";
const CONVERSATION: &str = "conversation:018f0f7b-6a4d-7a11-8d77-414243444546";
const OTHER_CONVERSATION: &str = "conversation:018f0f7b-6a4d-7a11-8d77-515253545556";
type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a disposable PostgreSQL 17 instance"]
#[allow(clippy::too_many_lines)]
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
            .add_service(WorkflowWatchFeasibilityServer::new(service.clone()))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    let endpoint = format!("http://{address}");
    let mut client = WorkflowWatchFeasibilityClient::connect(endpoint).await?;

    // Hold an update uncommitted while a cursorless watch selects its snapshot.
    // The old projection and boundary must be returned together; once committed,
    // the exact update must then be replayed even if its NOTIFY was missed.
    let (mut pending_client, pending_driver) = connect(&database_url).await?;
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
    let mut atomic_watch = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: None,
        })
        .await?
        .into_inner();
    let snapshot_frame = timeout(Duration::from_secs(2), atomic_watch.message())
        .await??
        .ok_or("atomic-boundary watch ended before its snapshot")?;
    let Some(watch_frame::Frame::Snapshot(boundary_snapshot)) = snapshot_frame.frame else {
        return Err("atomic-boundary watch did not begin with a snapshot".into());
    };
    assert_eq!(boundary_snapshot.revision, 0);
    assert_eq!(cursor_value(&boundary_snapshot.resume_cursor)?, 0);
    pending.commit().await?;
    let update_frame = timeout(Duration::from_secs(2), atomic_watch.message())
        .await??
        .ok_or("atomic-boundary watch lost the concurrent update")?;
    assert_eq!(frame_cursor(&update_frame)?, pending_cursor);
    let Some(watch_frame::Frame::Update(boundary_update)) = update_frame.frame else {
        return Err("atomic-boundary watch emitted a non-update frame".into());
    };
    assert_eq!(boundary_update.revision, 1);
    drop(atomic_watch);
    pending_driver.abort();
    reset_conversation(&database_url, CONVERSATION, "Feasibility conversation").await?;

    // Race writes against snapshots. Every observed revision must be exactly the
    // greatest revision at or before the transactionally selected cursor.
    let update_url = database_url.clone();
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
    let mut snapshot_latencies = Vec::with_capacity(110);
    for _ in 0..110 {
        let started = Instant::now();
        let snapshot = client
            .get_snapshot(GetSnapshotRequest {
                conversation_id: CONVERSATION.into(),
            })
            .await?
            .into_inner();
        snapshot_latencies.push(started.elapsed());
        let boundary = cursor_value(&snapshot.resume_cursor)?;
        let revision_at_boundary = revision_at_boundary(&database_url, boundary).await?;
        assert_eq!(snapshot.revision, revision_at_boundary);
    }
    updater.await??;

    // Cursorless watches start with one complete boundary snapshot and then emit
    // strictly later updates.
    let mut cursorless = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: None,
        })
        .await?
        .into_inner();
    let snapshot_frame = timeout(Duration::from_secs(2), cursorless.message())
        .await??
        .ok_or("cursorless watch ended before its snapshot")?;
    let Some(watch_frame::Frame::Snapshot(snapshot)) = snapshot_frame.frame else {
        return Err("cursorless watch did not begin with a snapshot".into());
    };
    let snapshot_cursor = cursor_value(&snapshot.resume_cursor)?;
    let (next_cursor, _) = append_update(
        &database_url,
        CONVERSATION,
        "after_snapshot",
        "visible",
        true,
    )
    .await?;
    let update_frame = timeout(Duration::from_secs(2), cursorless.message())
        .await??
        .ok_or("watch ended before its update")?;
    let Some(watch_frame::Frame::Update(update)) = update_frame.frame else {
        return Err("watch returned a non-update frame".into());
    };
    assert!(cursor_value(&update.cursor)? > snapshot_cursor);
    assert_eq!(cursor_value(&update.cursor)?, next_cursor);

    // Suppress NOTIFY entirely. Periodic table replay must still recover the row.
    let latest = client
        .get_snapshot(GetSnapshotRequest {
            conversation_id: CONVERSATION.into(),
        })
        .await?
        .into_inner();
    let mut lost_notification_watch = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: Some(latest.resume_cursor),
        })
        .await?
        .into_inner();
    let (lost_cursor, _) = append_update(
        &database_url,
        CONVERSATION,
        "lost_notification",
        "replayed",
        false,
    )
    .await?;
    let replayed = timeout(
        Duration::from_millis(500),
        lost_notification_watch.message(),
    )
    .await??
    .ok_or("lost notification was not recovered from the table")?;
    let Some(watch_frame::Frame::Update(replayed)) = replayed.frame else {
        return Err("lost-notification replay was not an update".into());
    };
    assert_eq!(cursor_value(&replayed.cursor)?, lost_cursor);

    // A scoped cursor cannot be used for another conversation.
    let Err(wrong_scope) = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: OTHER_CONVERSATION.into(),
            after_cursor: Some(replayed.cursor.clone()),
        })
        .await
    else {
        return Err("cross-conversation cursor unexpectedly succeeded".into());
    };
    assert_eq!(wrong_scope.code(), Code::InvalidArgument);

    // Advancing the synthetic retention watermark past an active cursor produces
    // typed expiry, while a cursorless request can always recover from a fresh snapshot.
    let (mut pruning_client, pruning_driver) = connect(&database_url).await?;
    let pruning = pruning_client.transaction().await?;
    let pruning_revision: i64 = pruning
        .query_one(
            "UPDATE workflow_watch_conversations SET revision = revision + 1 \
             WHERE id = $1 RETURNING revision",
            &[&CONVERSATION],
        )
        .await?
        .get(0);
    let pruned_ahead_cursor: i64 = pruning
        .query_one(
            "INSERT INTO workflow_watch_updates \
             (conversation_id, revision, kind, safe_payload) \
             VALUES ($1, $2, 'retention_race', 'pruned-before-replay') RETURNING cursor",
            &[&CONVERSATION, &pruning_revision],
        )
        .await?
        .get(0);
    pruning
        .execute(
            "DELETE FROM workflow_watch_updates WHERE conversation_id = $1 AND cursor <= $2",
            &[&CONVERSATION, &pruned_ahead_cursor],
        )
        .await?;
    pruning
        .execute(
            "UPDATE workflow_watch_watermarks SET pruned_through_cursor = $2 \
             WHERE conversation_id = $1",
            &[&CONVERSATION, &pruned_ahead_cursor],
        )
        .await?;
    pruning.commit().await?;
    pruning_driver.abort();
    let Err(active_expired) =
        timeout(Duration::from_secs(2), lost_notification_watch.message()).await?
    else {
        return Err("active watch silently crossed its retained watermark".into());
    };
    assert_eq!(active_expired.code(), Code::FailedPrecondition);
    assert_eq!(active_expired.message(), "WATCH_CURSOR_EXPIRED");
    let Err(expired) = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: Some(format!("v1|{CONVERSATION}|{}", lost_cursor - 1)),
        })
        .await
    else {
        return Err("expired cursor unexpectedly succeeded".into());
    };
    assert_eq!(expired.code(), Code::FailedPrecondition);
    assert_eq!(expired.message(), "WATCH_CURSOR_EXPIRED");
    let recovered_snapshot = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: None,
        })
        .await?
        .into_inner()
        .message()
        .await?
        .ok_or("snapshot recovery watch ended")?;
    assert!(matches!(
        recovered_snapshot.frame,
        Some(watch_frame::Frame::Snapshot(_))
    ));
    drop(cursorless);
    drop(lost_notification_watch);

    // Use a real Tonic connection with a deliberately small HTTP/2 receive window.
    // Stop polling it while enough durable updates fill the transport and the
    // one-item application queue; only that watcher may be evicted.
    let slow_service = WatchService::new(database_url.clone()).with_limits(
        1,
        Duration::from_millis(5),
        Duration::from_secs(60),
    );
    let slow_listener = TcpListener::bind("127.0.0.1:0").await?;
    let slow_address = slow_listener.local_addr()?;
    let slow_server = tokio::spawn(
        Server::builder()
            .add_service(WorkflowWatchFeasibilityServer::new(slow_service))
            .serve_with_incoming(TcpListenerStream::new(slow_listener)),
    );
    let slow_channel = Endpoint::from_shared(format!("http://{slow_address}"))?
        .initial_stream_window_size(1_024)
        .connect()
        .await?;
    let mut slow_client = WorkflowWatchFeasibilityClient::new(slow_channel);
    let current = client
        .get_snapshot(GetSnapshotRequest {
            conversation_id: CONVERSATION.into(),
        })
        .await?
        .into_inner();
    let slow_resume_cursor = current.resume_cursor;
    let mut healthy_stream = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: Some(slow_resume_cursor.clone()),
        })
        .await?
        .into_inner();
    let mut slow_stream = slow_client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: Some(slow_resume_cursor.clone()),
        })
        .await?
        .into_inner();
    let payload = "x".repeat(8 * 1_024);
    let mut backpressure_cursors = Vec::with_capacity(32);
    for index in 0..32 {
        let (committed_cursor, _) = append_update(
            &database_url,
            CONVERSATION,
            "backpressure",
            &format!("queued-{index}:{payload}"),
            true,
        )
        .await?;
        backpressure_cursors.push(committed_cursor);
    }
    sleep(Duration::from_millis(100)).await;
    let mut last_slow_cursor = cursor_value(&slow_resume_cursor)?;
    let eviction = loop {
        match timeout(Duration::from_secs(2), slow_stream.message()).await? {
            Ok(Some(frame)) => {
                let accepted_cursor = frame_cursor(&frame)?;
                assert!(accepted_cursor > last_slow_cursor);
                last_slow_cursor = accepted_cursor;
            }
            Err(status) => break status,
            Ok(None) => return Err("slow watcher ended without a status".into()),
        }
    };
    assert_eq!(eviction.code(), Code::ResourceExhausted);
    for expected_cursor in &backpressure_cursors {
        let frame = timeout(Duration::from_secs(2), healthy_stream.message())
            .await??
            .ok_or("healthy watcher ended during slow-reader test")?;
        assert_eq!(frame_cursor(&frame)?, *expected_cursor);
    }
    assert!(
        last_slow_cursor
            < *backpressure_cursors
                .last()
                .ok_or("backpressure fixture produced no updates")?
    );
    let mut resumed_stream = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: Some(format!("v1|{CONVERSATION}|{last_slow_cursor}")),
        })
        .await?
        .into_inner();
    for expected_cursor in backpressure_cursors
        .iter()
        .copied()
        .filter(|cursor| *cursor > last_slow_cursor)
    {
        let frame = timeout(Duration::from_secs(2), resumed_stream.message())
            .await??
            .ok_or("resumed slow watcher ended during replay")?;
        assert_eq!(frame_cursor(&frame)?, expected_cursor);
    }
    drop(healthy_stream);
    drop(resumed_stream);
    slow_server.abort();

    // Measure at least 100 warmed commit-to-watch samples against the pinned local
    // dependency. Correctness failure is independent of the percentile.
    let current = client
        .get_snapshot(GetSnapshotRequest {
            conversation_id: CONVERSATION.into(),
        })
        .await?
        .into_inner();
    let mut latency_watch = client
        .watch_conversation(WatchConversationRequest {
            conversation_id: CONVERSATION.into(),
            after_cursor: Some(current.resume_cursor),
        })
        .await?
        .into_inner();
    let mut watch_latencies = Vec::with_capacity(110);
    for index in 0..110_u64 {
        let (expected_cursor, _, started) = append_update_timed(
            &database_url,
            CONVERSATION,
            "latency",
            &format!("sample-{index}"),
            true,
        )
        .await?;
        let frame = timeout(Duration::from_secs(2), latency_watch.message())
            .await??
            .ok_or("latency watch ended")?;
        let Some(watch_frame::Frame::Update(update)) = frame.frame else {
            return Err("latency watch emitted a non-update frame".into());
        };
        assert_eq!(cursor_value(&update.cursor)?, expected_cursor);
        watch_latencies.push(started.elapsed());
    }
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

    server.abort();
    Ok(())
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
