//! Durable, scoped observation. Notifications only wake a replay of `PostgreSQL` rows.

use super::{StoreError, WorkflowStore, support};
use crate::model::{ConversationSnapshot, PresentationState, Timestamp};
use std::{
    future::{pending, poll_fn},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{MissedTickBehavior, interval, timeout},
};
use tokio_postgres::{AsyncMessage, Client, Config, IsolationLevel, NoTls, Transaction};
use tokio_stream::Stream;

const REPLAY_BATCH: i64 = 128;
const NOTIFICATION_CHANNEL: &str = "workflow_update";

#[derive(Clone, Debug)]
pub struct WatchOptions {
    pub buffer_capacity: usize,
    pub replay_interval: Duration,
    pub heartbeat_interval: Duration,
    pub slow_reader_timeout: Duration,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            buffer_capacity: 32,
            replay_interval: Duration::from_millis(100),
            heartbeat_interval: Duration::from_secs(30),
            slow_reader_timeout: Duration::from_secs(1),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConversationUpdate {
    pub update_id: String,
    pub activity_id: String,
    pub cursor: String,
    pub conversation_revision: u64,
    pub kind: String,
    pub affected_resource_ids: Vec<String>,
    pub presentation_state: PresentationState,
    pub waiting_on: String,
    pub safe_reason: String,
    pub accepted_at: Timestamp,
}

#[derive(Clone, Debug)]
pub enum WatchEvent {
    Snapshot(Box<ConversationSnapshot>),
    Update(ConversationUpdate),
    Heartbeat,
}

/// A bounded stream. Dropping it cancels only its observation tasks.
pub struct WatchStream {
    receiver: mpsc::Receiver<WatchEvent>,
    terminal: Arc<Mutex<Option<StoreError>>>,
    producer: JoinHandle<()>,
    terminated: bool,
}

impl Stream for WatchStream {
    type Item = Result<WatchEvent, StoreError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.terminated {
            return Poll::Ready(None);
        }
        // The producer retains its sender until after publishing its terminal
        // error. Poll the channel first so observing closure also observes that
        // error, even if the producer finishes during this poll.
        let received = self.receiver.poll_recv(context);
        let terminal = self
            .terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(error) = terminal {
            self.terminated = true;
            return Poll::Ready(Some(Err(error)));
        }
        if matches!(received, Poll::Ready(None)) {
            self.terminated = true;
        }
        received.map(|value| value.map(Ok))
    }
}

impl Drop for WatchStream {
    fn drop(&mut self) {
        self.producer.abort();
    }
}

impl WorkflowStore {
    /// Start observation from a scoped cursor, or an atomic authoritative snapshot.
    ///
    /// # Errors
    /// Returns invalid scope/cursor, expiry, database, or retained-state errors.
    pub async fn watch(
        &self,
        workspace_id: &str,
        project_id: &str,
        conversation_id: &str,
        after_cursor: Option<&str>,
        options: WatchOptions,
    ) -> Result<WatchStream, StoreError> {
        support::validate_scope(workspace_id, project_id, conversation_id)?;
        if options.buffer_capacity == 0
            || options.buffer_capacity > 128
            || options.replay_interval.is_zero()
            || options.heartbeat_interval.is_zero()
            || options.slow_reader_timeout.is_zero()
        {
            return Err(StoreError::InvalidInput("watch_options"));
        }
        // Parse before any lookup: another conversation's token reveals no scope.
        let after_cursor = after_cursor.filter(|value| !value.is_empty());
        let parsed = after_cursor
            .map(|value| parse_cursor(conversation_id, value))
            .transpose()?;
        let (sender, receiver) = mpsc::channel(options.buffer_capacity);
        let terminal = Arc::new(Mutex::new(None));
        let cursor = if let Some(after) = parsed {
            let (mut client, driver) = crate::connect(&self.database_url).await?;
            let _driver = AbortDriver(driver);
            let transaction = client
                .build_transaction()
                .isolation_level(IsolationLevel::RepeatableRead)
                .start()
                .await?;
            support::conversation(&transaction, workspace_id, project_id, conversation_id).await?;
            validate_cursor(&transaction, conversation_id, after).await?;
            transaction.commit().await?;
            after
        } else {
            let snapshot = self
                .get_snapshot(workspace_id, project_id, conversation_id)
                .await?;
            let boundary = parse_cursor(conversation_id, &snapshot.resume_cursor)?;
            sender
                .try_send(WatchEvent::Snapshot(Box::new(snapshot)))
                .map_err(|_| StoreError::Capacity("watch_buffer"))?;
            boundary
        };
        let producer = WatchProducer {
            store: self.clone(),
            conversation_id: conversation_id.to_owned(),
            cursor,
            sender,
            terminal: Arc::clone(&terminal),
            options,
            last_notification: None,
            caught_up: false,
        };
        let producer = tokio::spawn(producer.run());
        Ok(WatchStream {
            receiver,
            terminal,
            producer,
            terminated: false,
        })
    }

    /// Compact at most 1,000 expired deltas per conversation, for at most 32
    /// conversations. Watermarks and deletion commit together under the same lock
    /// used by authoritative writers. Current activity and all authority survive.
    ///
    /// # Errors
    /// Returns a database or retained-state error.
    pub async fn cleanup_observation(&self) -> Result<u64, StoreError> {
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let _driver = AbortDriver(driver);
        let candidates = client
            .query(
                "SELECT DISTINCT u.conversation_id FROM workflow_updates u \
             WHERE u.accepted_at < clock_timestamp() - interval '7 days' \
               AND NOT EXISTS (SELECT 1 FROM workflow_updates fresh \
                   WHERE fresh.conversation_id = u.conversation_id AND fresh.cursor <= u.cursor \
                     AND fresh.accepted_at >= clock_timestamp() - interval '7 days') \
             ORDER BY u.conversation_id LIMIT 32",
                &[],
            )
            .await?;
        let mut deleted = 0;
        for row in candidates {
            let conversation_id: String = row.get(0);
            let transaction = client.transaction().await?;
            transaction
                .query_one(
                    "SELECT id FROM workflow_conversations WHERE id = $1 FOR UPDATE",
                    &[&conversation_id],
                )
                .await?;
            let boundary: Option<i64> = transaction
                .query_one(
                    "SELECT max(cursor) FROM (SELECT cursor FROM workflow_updates \
                 WHERE conversation_id = $1 \
                   AND accepted_at < clock_timestamp() - interval '7 days' \
                   AND cursor < COALESCE((SELECT min(cursor) FROM workflow_updates \
                       WHERE conversation_id = $1 \
                         AND accepted_at >= clock_timestamp() - interval '7 days'), 9223372036854775807) \
                 ORDER BY cursor LIMIT 1000) expired",
                    &[&conversation_id],
                )
                .await?
                .get(0);
            if let Some(boundary) = boundary {
                transaction.execute(
                    "INSERT INTO workflow_watch_watermarks (conversation_id, pruned_through_cursor) \
                     VALUES ($1, $2) ON CONFLICT (conversation_id) DO UPDATE \
                     SET pruned_through_cursor = GREATEST(workflow_watch_watermarks.pruned_through_cursor, $2)",
                    &[&conversation_id, &boundary]).await?;
                deleted += transaction
                    .execute(
                        "DELETE FROM workflow_updates WHERE conversation_id = $1 AND cursor <= $2",
                        &[&conversation_id, &boundary],
                    )
                    .await?;
                transaction.execute(
                    "DELETE FROM workflow_activities WHERE id IN ( \
                     SELECT a.id FROM workflow_activities a WHERE a.conversation_id = $1 \
                       AND a.conversation_revision < (SELECT max(conversation_revision) \
                           FROM workflow_activities WHERE conversation_id = $1) \
                       AND NOT EXISTS (SELECT 1 FROM workflow_updates u WHERE u.activity_id = a.id) \
                     ORDER BY a.conversation_revision LIMIT 1000)", &[&conversation_id]).await?;
            }
            transaction.commit().await?;
        }
        Ok(deleted)
    }
}

/// Called inside every authoritative update transaction. Delivery happens only
/// after commit; rollback therefore cannot create false progress.
pub(super) async fn notify(
    transaction: &Transaction<'_>,
    conversation_id: &str,
) -> Result<(), StoreError> {
    transaction
        .query_one(
            "SELECT pg_notify($1, $2)",
            &[&NOTIFICATION_CHANNEL, &conversation_id],
        )
        .await?;
    Ok(())
}

struct WatchProducer {
    store: WorkflowStore,
    conversation_id: String,
    cursor: i64,
    sender: mpsc::Sender<WatchEvent>,
    terminal: Arc<Mutex<Option<StoreError>>>,
    options: WatchOptions,
    last_notification: Option<Instant>,
    caught_up: bool,
}

impl WatchProducer {
    async fn run(mut self) {
        if let Err(error) = self.observe().await {
            *self
                .terminal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error);
        }
    }

    async fn observe(&mut self) -> Result<(), StoreError> {
        let (mut client, driver) = crate::connect(&self.store.database_url).await?;
        let _driver = AbortDriver(driver);
        let mut listener = notification_connection(&self.store.database_url, &self.conversation_id)
            .await
            .ok();
        let mut replay_tick = interval(self.options.replay_interval);
        replay_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut heartbeat_tick = interval(self.options.heartbeat_interval);
        heartbeat_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        heartbeat_tick.tick().await;
        loop {
            self.replay(&mut client).await?;
            tokio::select! {
                _ = replay_tick.tick() => {
                    // Reconnect notification loss without making it an authority.
                    if listener.as_ref().is_none_or(|value| value.driver.0.is_finished()) {
                        listener = notification_connection(&self.store.database_url, &self.conversation_id).await.ok();
                    }
                }
                _ = heartbeat_tick.tick() => {
                    self.send(WatchEvent::Heartbeat).await?;
                }
                notification = receive_notification(&mut listener) => {
                    if notification {
                        crate::metrics::increment("workflow_watch_notifications_total");
                        self.last_notification = Some(Instant::now());
                    } else {
                        listener = None;
                    }
                }
                () = self.sender.closed() => return Ok(()),
            }
        }
    }

    async fn replay(&mut self, client: &mut Client) -> Result<(), StoreError> {
        loop {
            crate::metrics::increment("workflow_watch_replays_total");
            let transaction = client
                .build_transaction()
                .isolation_level(IsolationLevel::RepeatableRead)
                .start()
                .await?;
            validate_cursor(&transaction, &self.conversation_id, self.cursor).await?;
            let rows = transaction
                .query(
                    "SELECT id, activity_id, cursor, conversation_revision, kind, \
                        affected_resource_ids, presentation_state, waiting_on, safe_reason, \
                        workflow_ts_seconds(accepted_at), workflow_ts_nanos(accepted_at) \
                 FROM workflow_updates WHERE conversation_id = $1 AND cursor > $2 \
                 ORDER BY cursor LIMIT $3",
                    &[&self.conversation_id, &self.cursor, &REPLAY_BATCH],
                )
                .await?;
            transaction.commit().await?;
            if rows.is_empty() {
                self.caught_up = true;
                return Ok(());
            }
            let row_count = rows.len();
            for row in rows {
                let next_cursor: i64 = row.get(2);
                let accepted_at = Timestamp {
                    seconds: row.get(9),
                    nanos: i32::try_from(row.get::<_, i64>(10))
                        .map_err(|_| StoreError::CorruptState)?,
                };
                let update = ConversationUpdate {
                    update_id: row.get(0),
                    activity_id: row.get(1),
                    cursor: support::cursor(&self.conversation_id, next_cursor),
                    conversation_revision: support::unsigned(row.get(3))?,
                    kind: row.get(4),
                    affected_resource_ids: row.get(5),
                    presentation_state: PresentationState::parse(row.get(6))
                        .ok_or(StoreError::CorruptState)?,
                    waiting_on: row.get(7),
                    safe_reason: row.get(8),
                    accepted_at,
                };
                let terminal = matches!(
                    update.presentation_state,
                    PresentationState::Completed | PresentationState::Failed
                );
                self.send(WatchEvent::Update(update)).await?;
                crate::metrics::increment("workflow_watch_updates_queued_total");
                if let Some(notification) = self.last_notification.take() {
                    crate::metrics::observe(
                        "workflow_watch_notification_to_queue_seconds",
                        notification.elapsed(),
                    );
                }
                if let Some(age) = age(accepted_at) {
                    crate::metrics::observe("workflow_watch_commit_to_queue_seconds", age);
                    if self.caught_up {
                        crate::metrics::observe("workflow_watch_live_commit_to_queue_seconds", age);
                        if terminal {
                            crate::metrics::observe(
                                "workflow_watch_live_terminal_commit_to_queue_seconds",
                                age,
                            );
                        }
                    }
                }
                self.cursor = next_cursor;
            }
            if row_count < 128 {
                self.caught_up = true;
                return Ok(());
            }
        }
    }

    async fn send(&self, event: WatchEvent) -> Result<(), StoreError> {
        match timeout(self.options.slow_reader_timeout, self.sender.send(event)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(StoreError::Capacity("watch_closed")),
            Err(_) => {
                crate::metrics::increment("workflow_watch_slow_reader_total");
                Err(StoreError::Capacity("watch_buffer"))
            }
        }
    }
}

async fn validate_cursor(
    transaction: &Transaction<'_>,
    conversation_id: &str,
    after: i64,
) -> Result<(), StoreError> {
    let row = transaction
        .query_one(
            "SELECT COALESCE((SELECT pruned_through_cursor FROM workflow_watch_watermarks \
                         WHERE conversation_id = $1), 0), \
                GREATEST(COALESCE((SELECT max(cursor) FROM workflow_updates \
                                   WHERE conversation_id = $1), 0), \
                         COALESCE((SELECT pruned_through_cursor FROM workflow_watch_watermarks \
                                   WHERE conversation_id = $1), 0))",
            &[&conversation_id],
        )
        .await?;
    let watermark: i64 = row.get(0);
    if after < watermark {
        crate::metrics::increment("workflow_watch_expired_total");
        return Err(StoreError::WatchCursorExpired);
    }
    if after > row.get::<_, i64>(1) {
        return Err(StoreError::InvalidInput("after_cursor"));
    }
    Ok(())
}

fn parse_cursor(conversation_id: &str, value: &str) -> Result<i64, StoreError> {
    let prefix = format!("workflow-cursor-v1:{conversation_id}:");
    let number = value
        .strip_prefix(&prefix)
        .ok_or(StoreError::InvalidInput("after_cursor"))?;
    if number.is_empty()
        || number.len() > 19
        || !number.bytes().all(|byte| byte.is_ascii_digit())
        || (number.len() > 1 && number.starts_with('0'))
    {
        return Err(StoreError::InvalidInput("after_cursor"));
    }
    number
        .parse::<i64>()
        .map_err(|_| StoreError::InvalidInput("after_cursor"))
}

fn age(timestamp: Timestamp) -> Option<Duration> {
    let seconds = u64::try_from(timestamp.seconds).ok()?;
    let nanos = u32::try_from(timestamp.nanos).ok()?;
    let at = UNIX_EPOCH.checked_add(Duration::new(seconds, nanos))?;
    SystemTime::now().duration_since(at).ok()
}

struct AbortDriver(JoinHandle<()>);
impl Drop for AbortDriver {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct NotificationConnection {
    _client: Client,
    receiver: mpsc::Receiver<()>,
    driver: AbortDriver,
}

async fn notification_connection(
    database_url: &str,
    conversation_id: &str,
) -> Result<NotificationConnection, tokio_postgres::Error> {
    let mut config: Config = database_url.parse()?;
    config.options("-c search_path=pg_catalog,public,pg_temp");
    let (client, mut connection) = config.connect(NoTls).await?;
    let (sender, receiver) = mpsc::channel(16);
    let conversation_id = conversation_id.to_owned();
    let driver = AbortDriver(tokio::spawn(async move {
        while let Some(message) = poll_fn(|context| connection.poll_message(context)).await {
            match message {
                Ok(AsyncMessage::Notification(notification))
                    if notification.payload() == conversation_id =>
                {
                    let _ = sender.try_send(());
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }));
    client.batch_execute("LISTEN workflow_update").await?;
    Ok(NotificationConnection {
        _client: client,
        receiver,
        driver,
    })
}

async fn receive_notification(listener: &mut Option<NotificationConnection>) -> bool {
    if let Some(listener) = listener {
        listener.receiver.recv().await.is_some()
    } else {
        pending::<bool>().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn production_watch_queue_meets_healthy_internal_latency_bounds()
    -> Result<(), Box<dyn std::error::Error>> {
        let (sender, mut receiver) = mpsc::channel(32);
        let producer = WatchProducer {
            store: WorkflowStore::new("postgresql://bounded-fake"),
            conversation_id: "bounded-fake".into(),
            cursor: 0,
            sender,
            terminal: Arc::new(Mutex::new(None)),
            options: WatchOptions::default(),
            last_notification: None,
            caught_up: false,
        };
        let drain = tokio::spawn(async move {
            let mut count = 0;
            while receiver.recv().await.is_some() {
                count += 1;
            }
            count
        });
        let mut notifications = Vec::new();
        let mut terminals = Vec::new();
        for index in 0..105 {
            for (state, samples) in [
                (PresentationState::Processing, &mut notifications),
                (PresentationState::Completed, &mut terminals),
            ] {
                // A bounded fake supplies the already-accepted row; the owned
                // production queue operation is measured independently of storage.
                let update = ConversationUpdate {
                    update_id: "bounded-update".into(),
                    activity_id: "bounded-activity".into(),
                    cursor: "bounded-cursor".into(),
                    conversation_revision: 1,
                    kind: "turn_state_changed".into(),
                    affected_resource_ids: vec![],
                    presentation_state: state,
                    waiting_on: String::new(),
                    safe_reason: String::new(),
                    accepted_at: Timestamp {
                        seconds: 0,
                        nanos: 0,
                    },
                };
                let boundary = Instant::now();
                producer.send(WatchEvent::Update(update)).await?;
                if index >= 5 {
                    samples.push(boundary.elapsed());
                }
            }
        }
        for (name, mut samples) in [
            ("notification-to-queue", notifications),
            ("terminal-to-queue", terminals),
        ] {
            samples.sort();
            assert_eq!(samples.len(), 100);
            assert!(
                samples[94] <= Duration::from_millis(100),
                "{name}: p95={:?}",
                samples[94]
            );
            println!(
                "owned-watch-{name}: samples=100 p95-us={} max-us={}",
                samples[94].as_micros(),
                samples[99].as_micros()
            );
        }
        drop(producer);
        assert_eq!(drain.await?, 210);
        Ok(())
    }

    #[test]
    fn cursor_rejects_other_scope_noncanonical_and_overflow() {
        assert_eq!(
            parse_cursor("conversation-a", "workflow-cursor-v1:conversation-a:42").ok(),
            Some(42)
        );
        for value in [
            "workflow-cursor-v1:conversation-b:42",
            "workflow-cursor-v1:conversation-a:01",
            "workflow-cursor-v1:conversation-a:-1",
            "workflow-cursor-v1:conversation-a:9223372036854775808",
        ] {
            assert!(parse_cursor("conversation-a", value).is_err());
        }
    }
}
