//! Authenticated `JetStream` publication for immutable `PostgreSQL` workflow outbox bytes.
//!
//! `PostgreSQL` remains authoritative: broker acknowledgements only advance delivery
//! observations. Reports are `ACKed` only after their authoritative decision commits.

use crate::Configuration;
use async_nats::{ConnectOptions, HeaderMap, jetstream};
use ayni_orchestrator_events::{
    v1::ConversationTurnReportKind,
    workflow::{APPLICATION_SOURCE, decode_report},
};
use ayni_orchestrator_workflow_postgres::WorkflowStore;
use futures_util::StreamExt;
use std::{io, time::Duration};
use tokio::{sync::watch, time};
use tracing::{info, warn};

const OUTBOX_BATCH: u32 = 64;
const RETRY_DELAY: Duration = Duration::from_millis(250);
const REPORT_STREAM: &str = "ORCHESTRATOR_WORKFLOW_REPORT_V1";
const REPORT_CONSUMER: &str = "workflow-application";

/// Run authenticated publication and report acceptance until shutdown.
///
/// # Errors
/// Returns only a malformed or unavailable mounted application Secret.
pub async fn run_publisher(
    configuration: &Configuration,
    store: WorkflowStore,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), io::Error> {
    let password = configuration.broker_password()?;
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let Ok(client) = ConnectOptions::with_user_and_password(
            configuration.broker_username().to_owned(),
            password.clone(),
        )
        .connection_timeout(Duration::from_secs(3))
        .max_reconnects(None)
        .connect(configuration.broker_url())
        .await
        else {
            warn!(
                reason_code = "workflow_broker_unavailable",
                "workflow publication remains pending"
            );
            wait_or_shutdown(&mut shutdown).await;
            continue;
        };
        let mut context = jetstream::new(client);
        context.set_timeout(Duration::from_secs(3));
        let consumer_context = context.clone();
        let consumer_store = store.clone();
        let consumer_shutdown = shutdown.clone();
        tokio::spawn(async move {
            let mut shutdown = consumer_shutdown;
            loop {
                if *shutdown.borrow() {
                    return;
                }
                if let Err(error) = run_report_consumer(
                    consumer_context.clone(),
                    consumer_store.clone(),
                    shutdown.clone(),
                )
                .await
                {
                    warn!(reason_code="workflow_report_consumer_degraded", error=%error, "workflow reports remain pending");
                    wait_or_shutdown(&mut shutdown).await;
                }
            }
        });
        info!("authenticated workflow outbox publisher connected");
        loop {
            if *shutdown.borrow() {
                return Ok(());
            }
            match publish_pending(&context, &store).await {
                Ok(0) => wait_or_shutdown(&mut shutdown).await,
                Ok(_) => {}
                Err(error) => {
                    warn!(reason_code="workflow_outbox_publish_degraded", error=%error, "workflow outbox publication is pending recovery");
                    wait_or_shutdown(&mut shutdown).await;
                }
            }
        }
    }
}

async fn run_report_consumer(
    context: jetstream::Context,
    store: WorkflowStore,
    shutdown: watch::Receiver<bool>,
) -> Result<(), io::Error> {
    let consumer = context
        .get_consumer_from_stream(REPORT_CONSUMER, REPORT_STREAM)
        .await
        .map_err(|_| io::Error::other("workflow report consumer is unavailable"))?;
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let mut messages = consumer
            .batch()
            .max_messages(1)
            .expires(Duration::from_millis(500))
            .messages()
            .await
            .map_err(|_| io::Error::other("workflow report fetch failed"))?;
        while let Some(message) = messages.next().await {
            let message =
                message.map_err(|_| io::Error::other("workflow report delivery failed"))?;
            // Invalid reports are intentionally not ACKed: the broker's bounded
            // delivery policy retains evidence for operator handling.
            let (_, _, report) = decode_report(&message.payload)
                .map_err(|_| io::Error::other("invalid executor report"))?;
            let kind = ConversationTurnReportKind::try_from(report.kind)
                .map_err(|_| io::Error::other("invalid executor report"))?;
            match kind {
                ConversationTurnReportKind::Started => {
                    store
                        .accept_claim(&message.payload)
                        .await
                        .map_err(io::Error::other)?;
                }
                ConversationTurnReportKind::Completed | ConversationTurnReportKind::Failed => {
                    store
                        .accept_terminal_report(&message.payload)
                        .await
                        .map_err(io::Error::other)?;
                }
                ConversationTurnReportKind::Unspecified => {
                    return Err(io::Error::other("invalid executor report"));
                }
            }
            message
                .double_ack()
                .await
                .map_err(|_| io::Error::other("workflow report acknowledgement failed"))?;
        }
    }
}

async fn wait_or_shutdown(shutdown: &mut watch::Receiver<bool>) {
    tokio::select! { () = time::sleep(RETRY_DELAY) => {}, result = shutdown.changed() => { let _ = result; } }
}

async fn publish_pending(
    context: &jetstream::Context,
    store: &WorkflowStore,
) -> Result<u32, io::Error> {
    let records = store
        .pending_outbox(OUTBOX_BATCH)
        .await
        .map_err(io::Error::other)?;
    let count =
        u32::try_from(records.len()).map_err(|_| io::Error::other("outbox batch overflow"))?;
    for record in records {
        let mut headers = HeaderMap::new();
        headers.insert("Nats-Msg-Id", format!("{APPLICATION_SOURCE}:{}", record.id));
        context
            .publish_with_headers(
                record.subject.clone(),
                headers,
                record.event_bytes.clone().into(),
            )
            .await
            .map_err(|_| io::Error::other("workflow broker publish failed"))?
            .await
            .map_err(|_| io::Error::other("workflow broker storage acknowledgement failed"))?;
        store
            .mark_outbox_stored(&record.id, &record.event_bytes)
            .await
            .map_err(io::Error::other)?;
    }
    Ok(count)
}
