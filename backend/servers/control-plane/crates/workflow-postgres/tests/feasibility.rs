use ayni_orchestrator_workflow_postgres::{connect, migrate, migration_lock_id};
use std::error::Error;
use std::future::poll_fn;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Barrier, mpsc};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_postgres::{AsyncMessage, Client, Config, NoTls};

const DATABASE_URL_ENV: &str = "AYNI_POSTGRES_FEASIBILITY_URL";
type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a disposable PostgreSQL 17 instance"]
#[allow(clippy::too_many_lines)]
async fn postgres_feasibility_gate() -> TestResult {
    let database_url = std::env::var(DATABASE_URL_ENV)
        .map_err(|_| format!("set {DATABASE_URL_ENV} to a disposable PostgreSQL database"))?;
    let (mut client, driver) = connect(&database_url).await?;
    reset(&client).await?;

    // A held session lock must block migration before any DDL is visible.
    client
        .query_one("SELECT pg_advisory_lock($1)", &[&migration_lock_id()])
        .await?;
    let migration_url = database_url.clone();
    let mut blocked_migration = tokio::spawn(async move { migrate(&migration_url).await });
    assert!(
        timeout(Duration::from_millis(150), &mut blocked_migration)
            .await
            .is_err(),
        "migration bypassed the advisory lock"
    );
    let schema_visible: bool = client
        .query_one(
            "SELECT to_regclass('public.workflow_schema_migrations') IS NOT NULL",
            &[],
        )
        .await?
        .get(0);
    assert!(
        !schema_visible,
        "migration DDL became visible before the lock"
    );
    client
        .query_one("SELECT pg_advisory_unlock($1)", &[&migration_lock_id()])
        .await?;
    blocked_migration.await??;

    // Reset and synchronize fresh concurrent starters before any schema exists.
    reset(&client).await?;
    let starter_count = 8;
    let barrier = Arc::new(Barrier::new(starter_count));
    let mut migrations = Vec::new();
    for _ in 0..starter_count {
        let migration_url = database_url.clone();
        let barrier = Arc::clone(&barrier);
        migrations.push(tokio::spawn(async move {
            barrier.wait().await;
            migrate(&migration_url).await
        }));
    }
    for migration in migrations {
        migration.await??;
    }
    let applied: i64 = client
        .query_one("SELECT count(*) FROM workflow_schema_migrations", &[])
        .await?
        .get(0);
    assert_eq!(applied, 1);

    // A failed admission transaction cannot leave partial authority behind.
    let (mut rollback_client, rollback_driver) = connect(&database_url).await?;
    let transaction = rollback_client.transaction().await?;
    transaction
        .execute(
            "INSERT INTO workflow_gate_conversations (id) VALUES ($1)",
            &[&"rolled-back"],
        )
        .await?;
    transaction.rollback().await?;
    let persisted: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_gate_conversations WHERE id = $1",
            &[&"rolled-back"],
        )
        .await?
        .get(0);
    assert_eq!(persisted, 0);
    rollback_driver.abort();

    // Competing FIFO admissions serialize on the conversation row and receive
    // one contiguous sequence without a second eligible head.
    client
        .execute(
            "INSERT INTO workflow_gate_conversations (id) VALUES ($1)",
            &[&"conversation-race"],
        )
        .await?;
    let mut admissions = Vec::new();
    for index in 0..16_u64 {
        let admission_url = database_url.clone();
        admissions.push(tokio::spawn(async move {
            admit_turn(&admission_url, index).await
        }));
    }
    let mut allocated = Vec::new();
    for admission in admissions {
        allocated.push(admission.await??);
    }
    allocated.sort_unstable();
    assert_eq!(allocated, (1_i64..=16).collect::<Vec<_>>());

    // Exact immutable event bytes survive a fresh connection, including NUL and
    // non-UTF-8 values that would expose accidental text transcoding.
    let exact_bytes = vec![0, 1, 2, 127, 128, 254, 255];
    client
        .execute(
            "INSERT INTO workflow_gate_outbox (id, subject, event_bytes) VALUES ($1, $2, $3)",
            &[
                &"outbox-1",
                &"orchestrator.workflow.command.conversation.v1",
                &exact_bytes,
            ],
        )
        .await?;
    let (replay_client, replay_driver) = connect(&database_url).await?;
    let replayed: Vec<u8> = replay_client
        .query_one(
            "SELECT event_bytes FROM workflow_gate_outbox WHERE id = $1",
            &[&"outbox-1"],
        )
        .await?
        .get(0);
    assert_eq!(replayed, exact_bytes);
    replay_driver.abort();

    // A notification emitted while the listener is disconnected is recovered
    // from the durable table; a later notification wakes the live listener.
    let (offline_listener, offline_messages, offline_driver) =
        notification_connection(&database_url).await?;
    offline_listener
        .batch_execute("LISTEN workflow_gate_updates")
        .await?;
    drop(offline_listener);
    drop(offline_messages);
    offline_driver.abort();

    let first_cursor = append_event(&mut client, "lost-notification").await?;
    let (live_listener, mut live_messages, live_driver) =
        notification_connection(&database_url).await?;
    live_listener
        .batch_execute("LISTEN workflow_gate_updates")
        .await?;
    let recovered: Vec<i64> = live_listener
        .query(
            "SELECT cursor FROM workflow_gate_events WHERE cursor > $1 ORDER BY cursor",
            &[&0_i64],
        )
        .await?
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(recovered, vec![first_cursor]);

    let second_cursor = append_event(&mut client, "live-notification").await?;
    let payload = timeout(Duration::from_secs(2), async {
        loop {
            match live_messages.recv().await {
                Some(AsyncMessage::Notification(notification)) => {
                    break Ok::<_, Box<dyn Error + Send + Sync>>(notification.payload().to_owned());
                }
                Some(_) => {}
                None => break Err("notification connection ended".into()),
            }
        }
    })
    .await??;
    assert_eq!(payload, second_cursor.to_string());

    live_driver.abort();
    driver.abort();
    println!(
        "postgres-feasibility: advisory-lock=pass rollback=pass concurrent-migration=pass notification-replay=pass fifo-row-lock=pass exact-outbox=pass"
    );
    Ok(())
}

async fn reset(client: &Client) -> TestResult {
    client
        .batch_execute(
            "DROP TABLE IF EXISTS workflow_gate_turns CASCADE;\
             DROP TABLE IF EXISTS workflow_gate_conversations CASCADE;\
             DROP TABLE IF EXISTS workflow_gate_events CASCADE;\
             DROP TABLE IF EXISTS workflow_gate_outbox CASCADE;\
             DROP TABLE IF EXISTS workflow_schema_migrations CASCADE;",
        )
        .await?;
    Ok(())
}

async fn admit_turn(database_url: &str, index: u64) -> TestResult<i64> {
    let (mut client, driver) = connect(database_url).await?;
    let transaction = client.transaction().await?;
    let sequence: i64 = transaction
        .query_one(
            "SELECT next_sequence FROM workflow_gate_conversations WHERE id = $1 FOR UPDATE",
            &[&"conversation-race"],
        )
        .await?
        .get(0);
    transaction
        .execute(
            "UPDATE workflow_gate_conversations SET next_sequence = $2 WHERE id = $1",
            &[&"conversation-race", &(sequence + 1)],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO workflow_gate_turns (id, conversation_id, sequence) VALUES ($1, $2, $3)",
            &[&format!("turn-{index}"), &"conversation-race", &sequence],
        )
        .await?;
    transaction.commit().await?;
    driver.abort();
    Ok(sequence)
}

async fn append_event(client: &mut Client, kind: &str) -> TestResult<i64> {
    let transaction = client.transaction().await?;
    let cursor: i64 = transaction
        .query_one(
            "INSERT INTO workflow_gate_events (conversation_id, kind) VALUES ($1, $2) RETURNING cursor",
            &[&"conversation-race", &kind],
        )
        .await?
        .get(0);
    transaction
        .query_one(
            "SELECT pg_notify('workflow_gate_updates', $1)",
            &[&cursor.to_string()],
        )
        .await?;
    transaction.commit().await?;
    Ok(cursor)
}

async fn notification_connection(
    database_url: &str,
) -> TestResult<(Client, mpsc::Receiver<AsyncMessage>, JoinHandle<()>)> {
    let config: Config = database_url.parse()?;
    let (client, mut connection) = config.connect(NoTls).await?;
    let (sender, receiver) = mpsc::channel(16);
    let driver = tokio::spawn(async move {
        while let Some(message) = poll_fn(|context| connection.poll_message(context)).await {
            match message {
                Ok(message) => {
                    if sender.send(message).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    Ok((client, receiver, driver))
}
