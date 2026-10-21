//! Production `PostgreSQL` observation invariants, run against the disposable gate.
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, connect, migrate,
    store::observation::{WatchEvent, WatchOptions, WatchStream},
};
use std::{
    error::Error,
    time::{Duration, Instant},
};
use tokio::time::{sleep, timeout};
use tokio_postgres::{Client, Transaction};
use tokio_stream::StreamExt as _;
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a disposable PostgreSQL 17 instance"]
async fn postgres_observation_preserves_boundaries_replay_expiry_and_backpressure() -> TestResult {
    let url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    migrate(&url).await?;
    let store = WorkflowStore::new(url.clone());
    let nonce = Uuid::now_v7().to_string();
    let workspace = store
        .create_workspace(&format!("watch-workspace-{nonce}"), "Observation gate")
        .await?
        .resource;
    let project = store
        .create_project(
            &format!("watch-project-{nonce}"),
            &workspace.id,
            "Observation gate",
            "",
        )
        .await?
        .resource;
    let conversation = store
        .create_conversation(
            &format!("watch-conversation-{nonce}"),
            &workspace.id,
            &project.id,
            "Observation gate",
        )
        .await?
        .resource;
    let scope = Scope {
        workspace: workspace.id,
        project: project.id,
        conversation: conversation.id,
    };
    let (mut client, driver) = connect(&url).await?;
    prove_active_connection_loss(&url, &client, &scope).await?;
    prove_atomic_snapshot(&store, &mut client, &scope).await?;
    prove_notification_loss(&store, &mut client, &scope).await?;
    prove_reconnect_and_latency(&store, &mut client, &scope).await?;
    prove_slow_reader(&store, &mut client, &scope).await?;
    prove_expiry_and_snapshot_recovery(&store, &client, &scope).await?;
    driver.abort();
    println!(
        "production-watch: atomic-boundary=pass lost-notification=pass reconnect=pass scope=pass future-cursor=pass slow-reader=pass expiry=pass snapshot-recovery=pass current-activity-retained=pass"
    );
    Ok(())
}

struct Scope {
    workspace: String,
    project: String,
    conversation: String,
}

impl Scope {
    async fn watch(
        &self,
        store: &WorkflowStore,
        cursor: Option<&str>,
        options: WatchOptions,
    ) -> Result<WatchStream, StoreError> {
        store
            .watch(
                &self.workspace,
                &self.project,
                &self.conversation,
                cursor,
                options,
            )
            .await
    }
}

async fn next(watch: &mut WatchStream) -> TestResult<WatchEvent> {
    Ok(timeout(Duration::from_secs(3), watch.next())
        .await?
        .ok_or("watch ended")??)
}

async fn append(
    transaction: &Transaction<'_>,
    scope: &Scope,
    notify: bool,
) -> TestResult<(i64, i64)> {
    let revision: i64 = transaction.query_one("UPDATE workflow_conversations SET revision = revision + 1 WHERE id = $1 RETURNING revision", &[&scope.conversation]).await?.get(0);
    let activity = Uuid::now_v7().to_string();
    transaction.execute("INSERT INTO workflow_activities (id, workspace_id, project_id, conversation_id, conversation_revision, stage, presentation_state) VALUES ($1,$2,$3,$4,$5,'execution','processing')", &[&activity, &scope.workspace, &scope.project, &scope.conversation, &revision]).await?;
    let cursor: i64 = transaction.query_one("INSERT INTO workflow_updates (id, activity_id, workspace_id, project_id, conversation_id, conversation_revision, kind, affected_resource_ids, presentation_state) VALUES ($1,$2,$3,$4,$5,$6,'turn_state_changed',ARRAY[$5],'processing') RETURNING cursor", &[&Uuid::now_v7().to_string(), &activity, &scope.workspace, &scope.project, &scope.conversation, &revision]).await?.get(0);
    if notify {
        transaction
            .query_one(
                "SELECT pg_notify('workflow_update', $1)",
                &[&scope.conversation],
            )
            .await?;
    }
    Ok((cursor, revision))
}

async fn prove_atomic_snapshot(
    store: &WorkflowStore,
    client: &mut Client,
    scope: &Scope,
) -> TestResult {
    let pending = client.transaction().await?;
    let (_, pending_revision) = append(&pending, scope, true).await?;
    let mut watch = scope.watch(store, None, WatchOptions::default()).await?;
    let WatchEvent::Snapshot(snapshot) = next(&mut watch).await? else {
        return Err("missing atomic snapshot".into());
    };
    assert_eq!(
        snapshot.conversation_revision + 1,
        u64::try_from(pending_revision)?
    );
    pending.commit().await?;
    let WatchEvent::Update(update) = next(&mut watch).await? else {
        return Err("snapshot/commit gap".into());
    };
    assert_eq!(
        update.conversation_revision,
        u64::try_from(pending_revision)?
    );
    assert_ne!(snapshot.resume_cursor, update.cursor);
    drop(watch);
    let mut resumed = scope
        .watch(
            store,
            Some(&snapshot.resume_cursor),
            WatchOptions::default(),
        )
        .await?;
    let WatchEvent::Update(replay) = next(&mut resumed).await? else {
        return Err("replay missing".into());
    };
    assert_eq!(replay.cursor, update.cursor);
    assert_eq!(replay.update_id, update.update_id);
    assert!(matches!(
        scope
            .watch(
                store,
                Some("workflow-cursor-v1:another-conversation:1"),
                WatchOptions::default()
            )
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    let future = format!(
        "workflow-cursor-v1:{}:9223372036854775807",
        scope.conversation
    );
    assert!(matches!(
        scope
            .watch(store, Some(&future), WatchOptions::default())
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    Ok(())
}

async fn prove_notification_loss(
    store: &WorkflowStore,
    client: &mut Client,
    scope: &Scope,
) -> TestResult {
    let mut watch = scope.watch(store, None, WatchOptions::default()).await?;
    assert!(matches!(next(&mut watch).await?, WatchEvent::Snapshot(_)));
    // Let LISTEN and the immediate replay finish before omitting notification.
    sleep(Duration::from_millis(30)).await;
    let tx = client.transaction().await?;
    let (_, revision) = append(&tx, scope, false).await?;
    tx.commit().await?;
    let WatchEvent::Update(update) = next(&mut watch).await? else {
        return Err("notification loss did not replay".into());
    };
    assert_eq!(update.conversation_revision, u64::try_from(revision)?);
    Ok(())
}

async fn prove_reconnect_and_latency(
    store: &WorkflowStore,
    client: &mut Client,
    scope: &Scope,
) -> TestResult {
    let mut watch = scope.watch(store, None, WatchOptions::default()).await?;
    let WatchEvent::Snapshot(mut snapshot) = next(&mut watch).await? else {
        return Err("missing snapshot".into());
    };
    let mut samples = Vec::new();
    for index in 0..105 {
        let tx = client.transaction().await?;
        let (_, revision) = append(&tx, scope, true).await?;
        tx.commit().await?;
        let committed = Instant::now();
        let WatchEvent::Update(update) = next(&mut watch).await? else {
            return Err("missing update".into());
        };
        assert_eq!(update.conversation_revision, u64::try_from(revision)?);
        if index >= 5 {
            samples.push(committed.elapsed());
        }
        snapshot.resume_cursor = update.cursor;
        // Disconnect at each accepted state and replay the next committed row.
        drop(watch);
        watch = scope
            .watch(
                store,
                Some(&snapshot.resume_cursor),
                WatchOptions::default(),
            )
            .await?;
    }
    samples.sort();
    assert_eq!(samples.len(), 100);
    assert!(
        samples[94] <= Duration::from_millis(100),
        "watch p95={:?}",
        samples[94]
    );
    println!(
        "production-watch-commit-to-observed: samples=100 p50-us={} p95-us={} max-us={}",
        samples[49].as_micros(),
        samples[94].as_micros(),
        samples[99].as_micros()
    );
    Ok(())
}

async fn prove_slow_reader(
    store: &WorkflowStore,
    client: &mut Client,
    scope: &Scope,
) -> TestResult {
    let options = WatchOptions {
        buffer_capacity: 1,
        slow_reader_timeout: Duration::from_millis(25),
        ..WatchOptions::default()
    };
    let mut stalled = scope.watch(store, None, options).await?;
    // Keep the initial snapshot in its one-frame buffer while state progresses.
    let tx = client.transaction().await?;
    append(&tx, scope, true).await?;
    tx.commit().await?;
    sleep(Duration::from_millis(200)).await;
    assert!(matches!(
        stalled.next().await,
        Some(Err(StoreError::Capacity("watch_buffer")))
    ));
    let snapshot = store
        .get_snapshot(&scope.workspace, &scope.project, &scope.conversation)
        .await?;
    let mut healthy = scope
        .watch(
            store,
            Some(&snapshot.resume_cursor),
            WatchOptions::default(),
        )
        .await?;
    let tx = client.transaction().await?;
    let (_, revision) = append(&tx, scope, true).await?;
    tx.commit().await?;
    let WatchEvent::Update(update) = next(&mut healthy).await? else {
        return Err("slow reader blocked authority".into());
    };
    assert_eq!(update.conversation_revision, u64::try_from(revision)?);
    Ok(())
}

async fn prove_expiry_and_snapshot_recovery(
    store: &WorkflowStore,
    client: &Client,
    scope: &Scope,
) -> TestResult {
    let oldest: i64 = client
        .query_one(
            "SELECT min(cursor) FROM workflow_updates WHERE conversation_id = $1",
            &[&scope.conversation],
        )
        .await?
        .get(0);
    let old_cursor = format!("workflow-cursor-v1:{}:{oldest}", scope.conversation);
    // A clock reversal must never compact a fresh earlier cursor to reach an old later one.
    client.execute("UPDATE workflow_updates SET accepted_at = clock_timestamp() - interval '8 days' WHERE conversation_id = $1 AND cursor > $2", &[&scope.conversation, &oldest]).await?;
    let before: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_updates WHERE conversation_id = $1",
            &[&scope.conversation],
        )
        .await?
        .get(0);
    store.cleanup_observation().await?;
    let after: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_updates WHERE conversation_id = $1",
            &[&scope.conversation],
        )
        .await?
        .get(0);
    assert_eq!(before, after, "retention crossed an unexpired cursor");
    client.execute("UPDATE workflow_updates SET accepted_at = clock_timestamp() - interval '8 days' WHERE conversation_id = $1", &[&scope.conversation]).await?;
    assert!(store.cleanup_observation().await? > 0);
    assert!(matches!(
        scope
            .watch(store, Some(&old_cursor), WatchOptions::default())
            .await,
        Err(StoreError::WatchCursorExpired)
    ));
    let mut recovered = scope.watch(store, None, WatchOptions::default()).await?;
    let WatchEvent::Snapshot(snapshot) = next(&mut recovered).await? else {
        return Err("expiry recovery did not snapshot".into());
    };
    assert!(snapshot.current_activity.is_some());
    assert!(
        scope
            .watch(
                store,
                Some(&snapshot.resume_cursor),
                WatchOptions::default()
            )
            .await
            .is_ok()
    );
    Ok(())
}

async fn prove_active_connection_loss(url: &str, admin: &Client, scope: &Scope) -> TestResult {
    let separator = if url.contains('?') { '&' } else { '?' };
    let name = format!("observation_outage_{}", Uuid::now_v7().simple());
    let named = WorkflowStore::new(format!("{url}{separator}application_name={name}"));
    let mut watch = scope.watch(&named, None, WatchOptions::default()).await?;
    assert!(matches!(next(&mut watch).await?, WatchEvent::Snapshot(_)));
    // Confirm the replay connection is established before disrupting it.
    for _ in 0..100 {
        let count: i64 = admin
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE application_name = $1",
                &[&name],
            )
            .await?
            .get(0);
        if count >= 2 {
            break;
        }
        sleep(Duration::from_millis(5)).await;
    }
    admin
        .query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = $1",
            &[&name],
        )
        .await?;
    let terminal = timeout(Duration::from_secs(3), watch.next())
        .await?
        .ok_or("disrupted watch ended without error")?;
    assert!(matches!(terminal, Err(StoreError::Database(_))));
    let mut reconnected = scope.watch(&named, None, WatchOptions::default()).await?;
    assert!(matches!(
        next(&mut reconnected).await?,
        WatchEvent::Snapshot(_)
    ));
    Ok(())
}
