use ayni_orchestrator_workflow_postgres::{connect, migrate, migration_lock_id};
use std::error::Error;
use std::future::poll_fn;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Barrier, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_postgres::{AsyncMessage, Client, Config, NoTls};

const DATABASE_URL_ENV: &str = "AYNI_POSTGRES_FEASIBILITY_URL";
type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a disposable PostgreSQL 17 instance"]
async fn postgres_feasibility_gate() -> TestResult {
    let database_url = std::env::var(DATABASE_URL_ENV)
        .map_err(|_| format!("set {DATABASE_URL_ENV} to a disposable PostgreSQL database"))?;
    let (mut client, driver) = connect(&database_url).await?;
    reset(&client).await?;
    prove_advisory_lock(&client, &database_url).await?;
    reset(&client).await?;
    prove_concurrent_migrations(&client, &database_url).await?;
    prove_transaction_rollback(&client, &database_url).await?;
    prove_fifo_allocation(&client, &database_url).await?;
    prove_exact_outbox_bytes(&client, &database_url).await?;
    prove_notification_replay(&mut client, &database_url).await?;
    driver.abort();
    println!(
        "postgres-feasibility: advisory-lock=pass rollback=pass concurrent-migration=pass notification-replay=pass fifo-row-lock=pass exact-outbox=pass"
    );
    Ok(())
}

async fn prove_advisory_lock(client: &Client, database_url: &str) -> TestResult {
    client
        .query_one("SELECT pg_advisory_lock($1)", &[&migration_lock_id()])
        .await?;
    let application_name = format!("ayni_m1_migration_lock_probe_{}", std::process::id());
    let migration_url = database_url_with_application_name(database_url, &application_name);
    let blocked_migration = tokio::spawn(async move { migrate(&migration_url).await });
    let waiting_pid = wait_for_advisory_lock_wait(
        client,
        &application_name,
        migration_lock_id(),
        &blocked_migration,
    )
    .await?;
    assert!(
        !blocked_migration.is_finished(),
        "migration backend {waiting_pid} completed while its advisory lock was not granted"
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
    Ok(())
}

async fn prove_concurrent_migrations(client: &Client, database_url: &str) -> TestResult {
    let starter_count = 8;
    let barrier = Arc::new(Barrier::new(starter_count));
    let mut migrations = Vec::new();
    for _ in 0..starter_count {
        let migration_url = database_url.to_owned();
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
    Ok(())
}

async fn prove_transaction_rollback(client: &Client, database_url: &str) -> TestResult {
    let (mut rollback_client, rollback_driver) = connect(database_url).await?;
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
    Ok(())
}

async fn prove_fifo_allocation(client: &Client, database_url: &str) -> TestResult {
    client
        .execute(
            "INSERT INTO workflow_gate_conversations (id) VALUES ($1)",
            &[&"conversation-race"],
        )
        .await?;
    let mut admissions = Vec::new();
    for index in 0..16_u64 {
        let admission_url = database_url.to_owned();
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
    Ok(())
}

async fn prove_exact_outbox_bytes(client: &Client, database_url: &str) -> TestResult {
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
    let (replay_client, replay_driver) = connect(database_url).await?;
    let replayed: Vec<u8> = replay_client
        .query_one(
            "SELECT event_bytes FROM workflow_gate_outbox WHERE id = $1",
            &[&"outbox-1"],
        )
        .await?
        .get(0);
    assert_eq!(replayed, exact_bytes);
    replay_driver.abort();
    Ok(())
}

async fn prove_notification_replay(client: &mut Client, database_url: &str) -> TestResult {
    let (offline_listener, offline_messages, offline_driver) =
        notification_connection(database_url).await?;
    offline_listener
        .batch_execute("LISTEN workflow_gate_updates")
        .await?;
    drop(offline_listener);
    drop(offline_messages);
    offline_driver.abort();

    let first_cursor = append_event(client, "lost-notification").await?;
    let (live_listener, mut live_messages, live_driver) =
        notification_connection(database_url).await?;
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

    let second_cursor = append_event(client, "live-notification").await?;
    let payload = receive_notification(&mut live_messages).await?;
    assert_eq!(payload, second_cursor.to_string());
    live_driver.abort();
    Ok(())
}

async fn receive_notification(messages: &mut mpsc::Receiver<AsyncMessage>) -> TestResult<String> {
    timeout(Duration::from_secs(2), async {
        loop {
            match messages.recv().await {
                Some(AsyncMessage::Notification(notification)) => {
                    break Ok(notification.payload().to_owned());
                }
                Some(_) => {}
                None => break Err("notification connection ended".into()),
            }
        }
    })
    .await?
}

fn database_url_with_application_name(database_url: &str, application_name: &str) -> String {
    let separator = if database_url.contains('?') { '&' } else { '?' };
    format!("{database_url}{separator}application_name={application_name}")
}

async fn wait_for_advisory_lock_wait(
    client: &Client,
    application_name: &str,
    lock_id: i64,
    migration: &JoinHandle<Result<(), tokio_postgres::Error>>,
) -> TestResult<i32> {
    timeout(Duration::from_secs(5), async {
        loop {
            if migration.is_finished() {
                return Err("migration completed before waiting on the advisory lock".into());
            }
            let waiting = client
                .query_opt(
                    "SELECT activity.pid \
                     FROM pg_stat_activity activity \
                     JOIN pg_locks lock ON lock.pid = activity.pid \
                     WHERE activity.application_name = $1 \
                       AND lock.locktype = 'advisory' \
                       AND NOT lock.granted \
                       AND lock.classid::bigint = (($2::bigint >> 32) & 4294967295) \
                       AND lock.objid::bigint = ($2::bigint & 4294967295) \
                       AND lock.objsubid = 1",
                    &[&application_name, &lock_id],
                )
                .await?;
            if let Some(row) = waiting {
                return Ok(row.get(0));
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "migration did not reach the expected advisory-lock wait within five seconds")?
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
