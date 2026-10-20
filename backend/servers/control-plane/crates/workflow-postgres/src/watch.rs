//! Isolated PostgreSQL/Tonic snapshot and watch feasibility service.
//!
//! This module is not registered by the control plane. It exists to exercise the
//! M1 cursor and backpressure contract before runtime authority is approved.

use crate::connect;
use crate::watch_v1::workflow_watch_feasibility_server::WorkflowWatchFeasibility;
use crate::watch_v1::{
    ConversationSnapshot, ConversationUpdate, GetSnapshotRequest, Heartbeat,
    WatchConversationRequest, WatchFrame, watch_frame,
};
use std::future::{pending, poll_fn};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};
use tokio_postgres::{AsyncMessage, Client, Config, IsolationLevel, NoTls};
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

const REPLAY_BATCH: i64 = 128;

#[derive(Clone, Debug)]
pub struct WatchService {
    database_url: Arc<str>,
    buffer_capacity: usize,
    replay_interval: Duration,
    heartbeat_interval: Duration,
}

impl WatchService {
    #[must_use]
    pub fn new(database_url: impl Into<Arc<str>>) -> Self {
        Self {
            database_url: database_url.into(),
            buffer_capacity: 32,
            replay_interval: Duration::from_millis(25),
            heartbeat_interval: Duration::from_secs(30),
        }
    }

    #[must_use]
    pub const fn with_limits(
        mut self,
        buffer_capacity: usize,
        replay_interval: Duration,
        heartbeat_interval: Duration,
    ) -> Self {
        self.buffer_capacity = buffer_capacity;
        self.replay_interval = replay_interval;
        self.heartbeat_interval = heartbeat_interval;
        self
    }

    async fn snapshot(&self, conversation_id: &str) -> Result<ConversationSnapshot, Status> {
        validate_conversation_id(conversation_id)?;
        let (mut client, driver) = connect(&self.database_url)
            .await
            .map_err(database_unavailable)?;
        let transaction = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .start()
            .await
            .map_err(database_unavailable)?;
        let conversation = transaction
            .query_opt(
                "SELECT revision, title FROM workflow_watch_conversations WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(database_unavailable)?
            .ok_or_else(|| Status::not_found("conversation not found"))?;
        let revision: i64 = conversation.get(0);
        let title: String = conversation.get(1);
        let boundary: i64 = transaction
            .query_one(
                "SELECT GREATEST(\
                    COALESCE((SELECT max(cursor) FROM workflow_watch_updates WHERE conversation_id = $1), 0), \
                    COALESCE((SELECT pruned_through_cursor FROM workflow_watch_watermarks WHERE conversation_id = $1), 0)\
                 )",
                &[&conversation_id],
            )
            .await
            .map_err(database_unavailable)?
            .get(0);
        transaction.commit().await.map_err(database_unavailable)?;
        driver.abort();
        Ok(ConversationSnapshot {
            conversation_id: conversation_id.to_owned(),
            revision: unsigned(revision)?,
            title,
            resume_cursor: cursor(conversation_id, boundary),
        })
    }

    async fn validate_resume_cursor(
        &self,
        conversation_id: &str,
        after_cursor: &str,
    ) -> Result<i64, Status> {
        let after = parse_cursor(conversation_id, after_cursor)?;
        let (client, driver) = connect(&self.database_url)
            .await
            .map_err(database_unavailable)?;
        let row = client
            .query_opt(
                "SELECT c.revision, COALESCE(w.pruned_through_cursor, 0), \
                 GREATEST(\
                    COALESCE(w.pruned_through_cursor, 0), \
                    COALESCE((SELECT max(cursor) FROM workflow_watch_updates u WHERE u.conversation_id = c.id), 0)\
                 ) \
                 FROM workflow_watch_conversations c \
                 LEFT JOIN workflow_watch_watermarks w ON w.conversation_id = c.id \
                 WHERE c.id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(database_unavailable)?
            .ok_or_else(|| Status::not_found("conversation not found"))?;
        let pruned_through: i64 = row.get(1);
        let newest: i64 = row.get(2);
        driver.abort();
        if after < pruned_through {
            return Err(Status::failed_precondition("WATCH_CURSOR_EXPIRED"));
        }
        if after > newest {
            return Err(Status::invalid_argument(
                "cursor is ahead of retained history",
            ));
        }
        Ok(after)
    }
}

#[tonic::async_trait]
impl WorkflowWatchFeasibility for WatchService {
    async fn get_snapshot(
        &self,
        request: Request<GetSnapshotRequest>,
    ) -> Result<Response<ConversationSnapshot>, Status> {
        Ok(Response::new(
            self.snapshot(&request.into_inner().conversation_id).await?,
        ))
    }

    type WatchConversationStream =
        Pin<Box<dyn Stream<Item = Result<WatchFrame, Status>> + Send + 'static>>;

    async fn watch_conversation(
        &self,
        request: Request<WatchConversationRequest>,
    ) -> Result<Response<Self::WatchConversationStream>, Status> {
        let request = request.into_inner();
        validate_conversation_id(&request.conversation_id)?;
        let initial_snapshot = if request.after_cursor.is_none() {
            Some(self.snapshot(&request.conversation_id).await?)
        } else {
            None
        };
        let after = if let Some(after_cursor) = request.after_cursor.as_deref() {
            self.validate_resume_cursor(&request.conversation_id, after_cursor)
                .await?
        } else {
            parse_cursor(
                &request.conversation_id,
                &initial_snapshot
                    .as_ref()
                    .ok_or_else(|| Status::internal("missing initial snapshot"))?
                    .resume_cursor,
            )?
        };

        let capacity = self.buffer_capacity.max(1);
        let (sender, receiver) = mpsc::channel(capacity);
        let terminal = Arc::new(Mutex::new(None));
        if let Some(snapshot) = initial_snapshot {
            sender
                .try_send(Ok(WatchFrame {
                    frame: Some(watch_frame::Frame::Snapshot(snapshot)),
                }))
                .map_err(|_| Status::resource_exhausted("watch buffer unavailable"))?;
        }
        let producer = WatchProducer {
            database_url: Arc::clone(&self.database_url),
            conversation_id: request.conversation_id,
            cursor: after,
            sender,
            terminal: Arc::clone(&terminal),
            replay_interval: self.replay_interval,
            heartbeat_interval: self.heartbeat_interval,
        };
        tokio::spawn(producer.run());
        Ok(Response::new(Box::pin(EvictingStream {
            receiver,
            terminal,
            terminated: false,
        })))
    }
}

struct WatchProducer {
    database_url: Arc<str>,
    conversation_id: String,
    cursor: i64,
    sender: mpsc::Sender<Result<WatchFrame, Status>>,
    terminal: Arc<Mutex<Option<Status>>>,
    replay_interval: Duration,
    heartbeat_interval: Duration,
}

impl WatchProducer {
    async fn run(mut self) {
        let listener = notification_connection(&self.database_url).await.ok();
        let mut notification_receiver = listener.as_ref().map(NotificationConnection::receiver);
        let mut replay_tick = interval(self.replay_interval);
        replay_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut heartbeat_tick = interval(self.heartbeat_interval);
        heartbeat_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // Consume the immediate ticks; frames should be driven by replay first.
        replay_tick.tick().await;
        heartbeat_tick.tick().await;

        loop {
            if let Err(status) = self.replay().await {
                self.set_terminal(status);
                return;
            }
            tokio::select! {
                _ = replay_tick.tick() => {}
                _ = heartbeat_tick.tick() => {
                    if !self.send(WatchFrame {
                        frame: Some(watch_frame::Frame::Heartbeat(Heartbeat {})),
                    }) {
                        return;
                    }
                }
                notification_open = receive_notification(&mut notification_receiver) => {
                    if !notification_open {
                        notification_receiver = None;
                    }
                }
                () = self.sender.closed() => return,
            }
        }
    }

    async fn replay(&mut self) -> Result<(), Status> {
        loop {
            let (mut client, driver) = connect(&self.database_url)
                .await
                .map_err(database_unavailable)?;
            let transaction = client
                .build_transaction()
                .isolation_level(IsolationLevel::RepeatableRead)
                .start()
                .await
                .map_err(database_unavailable)?;
            let watermark: i64 = transaction
                .query_one(
                    "SELECT COALESCE(pruned_through_cursor, 0) \
                     FROM workflow_watch_watermarks WHERE conversation_id = $1",
                    &[&self.conversation_id],
                )
                .await
                .map_err(database_unavailable)?
                .get(0);
            if self.cursor < watermark {
                transaction.rollback().await.map_err(database_unavailable)?;
                driver.abort();
                return Err(Status::failed_precondition("WATCH_CURSOR_EXPIRED"));
            }
            let rows = transaction
                .query(
                    "SELECT cursor, revision, kind, safe_payload \
                     FROM workflow_watch_updates \
                     WHERE conversation_id = $1 AND cursor > $2 \
                     ORDER BY cursor LIMIT $3",
                    &[&self.conversation_id, &self.cursor, &REPLAY_BATCH],
                )
                .await
                .map_err(database_unavailable)?;
            transaction.commit().await.map_err(database_unavailable)?;
            driver.abort();
            if rows.is_empty() {
                return Ok(());
            }
            let row_count = rows.len();
            for row in rows {
                let next_cursor: i64 = row.get(0);
                let revision: i64 = row.get(1);
                let update = ConversationUpdate {
                    revision: unsigned(revision)?,
                    kind: row.get(2),
                    safe_payload: row.get(3),
                    cursor: cursor(&self.conversation_id, next_cursor),
                };
                if !self.send(WatchFrame {
                    frame: Some(watch_frame::Frame::Update(update)),
                }) {
                    return Err(Status::resource_exhausted(
                        "watch reader exceeded its bounded buffer",
                    ));
                }
                self.cursor = next_cursor;
            }
            if row_count
                < usize::try_from(REPLAY_BATCH)
                    .map_err(|_| Status::internal("invalid replay batch"))?
            {
                return Ok(());
            }
        }
    }

    fn send(&self, frame: WatchFrame) -> bool {
        match self.sender.try_send(Ok(frame)) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.set_terminal(Status::resource_exhausted(
                    "watch reader exceeded its bounded buffer",
                ));
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    fn set_terminal(&self, status: Status) {
        let mut terminal = self
            .terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if terminal.is_none() {
            *terminal = Some(status);
        }
    }
}

struct EvictingStream {
    receiver: mpsc::Receiver<Result<WatchFrame, Status>>,
    terminal: Arc<Mutex<Option<Status>>>,
    terminated: bool,
}

impl Stream for EvictingStream {
    type Item = Result<WatchFrame, Status>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.terminated {
            return Poll::Ready(None);
        }
        let terminal = {
            self.terminal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
        };
        if let Some(status) = terminal {
            self.terminated = true;
            return Poll::Ready(Some(Err(status)));
        }
        Pin::new(&mut self.receiver).poll_recv(context)
    }
}

struct NotificationConnection {
    _client: Client,
    receiver: tokio::sync::Mutex<mpsc::Receiver<()>>,
    driver: JoinHandle<()>,
}

impl NotificationConnection {
    const fn receiver(&self) -> &tokio::sync::Mutex<mpsc::Receiver<()>> {
        &self.receiver
    }
}

impl Drop for NotificationConnection {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

async fn notification_connection(
    database_url: &str,
) -> Result<NotificationConnection, tokio_postgres::Error> {
    let config: Config = database_url.parse()?;
    let (client, mut connection) = config.connect(NoTls).await?;
    let (sender, receiver) = mpsc::channel(16);
    let driver = tokio::spawn(async move {
        while let Some(message) = poll_fn(|context| connection.poll_message(context)).await {
            match message {
                Ok(AsyncMessage::Notification(_)) => {
                    if sender.try_send(()).is_err() && sender.is_closed() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    if let Err(error) = client.batch_execute("LISTEN workflow_watch_update").await {
        driver.abort();
        return Err(error);
    }
    Ok(NotificationConnection {
        _client: client,
        receiver: tokio::sync::Mutex::new(receiver),
        driver,
    })
}

async fn receive_notification(
    receiver: &mut Option<&tokio::sync::Mutex<mpsc::Receiver<()>>>,
) -> bool {
    if let Some(receiver) = receiver {
        receiver.lock().await.recv().await.is_some()
    } else {
        pending::<bool>().await
    }
}

fn cursor(conversation_id: &str, value: i64) -> String {
    format!("v1|{conversation_id}|{value}")
}

fn parse_cursor(conversation_id: &str, value: &str) -> Result<i64, Status> {
    let mut parts = value.split('|');
    let version = parts.next();
    let scope = parts.next();
    let cursor = parts.next();
    if version != Some("v1") || scope != Some(conversation_id) || parts.next().is_some() {
        return Err(Status::invalid_argument("invalid conversation cursor"));
    }
    let cursor = cursor
        .ok_or_else(|| Status::invalid_argument("invalid conversation cursor"))?
        .parse::<i64>()
        .map_err(|_| Status::invalid_argument("invalid conversation cursor"))?;
    if cursor < 0 {
        return Err(Status::invalid_argument("invalid conversation cursor"));
    }
    Ok(cursor)
}

fn validate_conversation_id(value: &str) -> Result<(), Status> {
    if value.len() > 128 || !value.strip_prefix("conversation:").is_some_and(uuid_like) {
        return Err(Status::invalid_argument("invalid conversation ID"));
    }
    Ok(())
}

fn uuid_like(value: &str) -> bool {
    value.len() == 36
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

fn unsigned(value: i64) -> Result<u64, Status> {
    u64::try_from(value).map_err(|_| Status::internal("negative durable sequence"))
}

fn database_unavailable(_: tokio_postgres::Error) -> Status {
    Status::unavailable("workflow PostgreSQL unavailable")
}

/// Create or reset one synthetic conversation used only by the feasibility gate.
///
/// # Errors
/// Returns `PostgreSQL` connection or transaction errors.
pub async fn reset_conversation(
    database_url: &str,
    conversation_id: &str,
    title: &str,
) -> Result<(), tokio_postgres::Error> {
    let (mut client, driver) = connect(database_url).await?;
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS public.workflow_watch_conversations (\
                 id text PRIMARY KEY,\
                 title text NOT NULL,\
                 revision bigint NOT NULL DEFAULT 0 CHECK (revision >= 0)\
             );\
             CREATE TABLE IF NOT EXISTS public.workflow_watch_updates (\
                 cursor bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,\
                 conversation_id text NOT NULL REFERENCES public.workflow_watch_conversations(id) ON DELETE CASCADE,\
                 revision bigint NOT NULL CHECK (revision > 0),\
                 kind text NOT NULL,\
                 safe_payload text NOT NULL,\
                 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),\
                 UNIQUE (conversation_id, revision)\
             );\
             CREATE INDEX IF NOT EXISTS workflow_watch_updates_replay \
                 ON public.workflow_watch_updates (conversation_id, cursor);\
             CREATE TABLE IF NOT EXISTS public.workflow_watch_watermarks (\
                 conversation_id text PRIMARY KEY REFERENCES public.workflow_watch_conversations(id) ON DELETE CASCADE,\
                 pruned_through_cursor bigint NOT NULL DEFAULT 0 CHECK (pruned_through_cursor >= 0)\
             );",
        )
        .await?;
    let transaction = client.transaction().await?;
    transaction
        .execute(
            "DELETE FROM workflow_watch_conversations WHERE id = $1",
            &[&conversation_id],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO workflow_watch_conversations (id, title) VALUES ($1, $2)",
            &[&conversation_id, &title],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO workflow_watch_watermarks (conversation_id) VALUES ($1)",
            &[&conversation_id],
        )
        .await?;
    transaction.commit().await?;
    driver.abort();
    Ok(())
}

/// Atomically update the conversation projection, append its replay row, and
/// optionally emit the lossy LISTEN/NOTIFY wake-up.
///
/// # Errors
/// Returns `PostgreSQL` connection or transaction errors.
pub async fn append_update(
    database_url: &str,
    conversation_id: &str,
    kind: &str,
    safe_payload: &str,
    notify: bool,
) -> Result<(i64, i64), tokio_postgres::Error> {
    let (cursor, revision, _) =
        append_update_timed(database_url, conversation_id, kind, safe_payload, notify).await?;
    Ok((cursor, revision))
}

/// Append a synthetic update and capture the local instant immediately after the
/// database acknowledges `COMMIT` for feasibility latency measurement.
///
/// # Errors
/// Returns `PostgreSQL` connection or transaction errors.
pub async fn append_update_timed(
    database_url: &str,
    conversation_id: &str,
    kind: &str,
    safe_payload: &str,
    notify: bool,
) -> Result<(i64, i64, Instant), tokio_postgres::Error> {
    let (mut client, driver) = connect(database_url).await?;
    let transaction = client.transaction().await?;
    let revision: i64 = transaction
        .query_one(
            "UPDATE workflow_watch_conversations SET revision = revision + 1 \
             WHERE id = $1 RETURNING revision",
            &[&conversation_id],
        )
        .await?
        .get(0);
    let cursor: i64 = transaction
        .query_one(
            "INSERT INTO workflow_watch_updates \
             (conversation_id, revision, kind, safe_payload) VALUES ($1, $2, $3, $4) \
             RETURNING cursor",
            &[&conversation_id, &revision, &kind, &safe_payload],
        )
        .await?
        .get(0);
    if notify {
        transaction
            .query_one(
                "SELECT pg_notify('workflow_watch_update', $1)",
                &[&conversation_id],
            )
            .await?;
    }
    transaction.commit().await?;
    let committed_at = Instant::now();
    driver.abort();
    Ok((cursor, revision, committed_at))
}

/// Prune replay rows and move the synthetic seven-day watermark through a cursor.
///
/// # Errors
/// Returns `PostgreSQL` connection or transaction errors.
pub async fn prune_through(
    database_url: &str,
    conversation_id: &str,
    through_cursor: i64,
) -> Result<(), tokio_postgres::Error> {
    let (mut client, driver) = connect(database_url).await?;
    let transaction = client.transaction().await?;
    transaction
        .execute(
            "DELETE FROM workflow_watch_updates WHERE conversation_id = $1 AND cursor <= $2",
            &[&conversation_id, &through_cursor],
        )
        .await?;
    transaction
        .execute(
            "UPDATE workflow_watch_watermarks \
             SET pruned_through_cursor = GREATEST(pruned_through_cursor, $2) \
             WHERE conversation_id = $1",
            &[&conversation_id, &through_cursor],
        )
        .await?;
    transaction.commit().await?;
    driver.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{EvictingStream, WatchFrame};
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;
    use tokio_stream::StreamExt as _;
    use tonic::Code;

    #[tokio::test]
    async fn terminal_status_preempts_buffered_data() {
        let (sender, receiver) = mpsc::channel(1);
        assert!(sender.try_send(Ok(WatchFrame { frame: None })).is_ok());
        let terminal = Arc::new(Mutex::new(Some(tonic::Status::unavailable(
            "workflow PostgreSQL unavailable",
        ))));
        let mut stream = EvictingStream {
            receiver,
            terminal,
            terminated: false,
        };

        let Some(Err(status)) = stream.next().await else {
            panic!("terminal status was hidden behind buffered data");
        };
        assert_eq!(status.code(), Code::Unavailable);
        assert!(stream.next().await.is_none());
    }
}
