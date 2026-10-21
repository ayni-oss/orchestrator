//! Authenticated `JetStream` publication for immutable `PostgreSQL` workflow outbox bytes.
//!
//! `PostgreSQL` remains authoritative: broker acknowledgements only advance delivery
//! observations. Reports are `ACKed` only after their authoritative decision commits.

use crate::Configuration;
use async_nats::{ConnectOptions, HeaderMap, jetstream};
use ayni_orchestrator_events::{
    v1::ConversationTurnReportKind,
    workflow::{
        APPLICATION_SOURCE, decode_report,
        transport::{ConsumerRole, rejected_delivery},
    },
};
use ayni_orchestrator_workflow_postgres::{WorkflowStore, metrics};
use futures_util::StreamExt;
use std::{error::Error as _, io, time::Duration};
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
        .request_timeout(Some(Duration::from_secs(3)))
        .max_reconnects(None)
        .connect(configuration.broker_url())
        .await
        else {
            warn!(
                reason_code = "workflow_broker_unavailable",
                "workflow publication remains pending"
            );
            record_connection_failure(&store).await;
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
        tokio::spawn(run_broker_observer(
            context.clone(),
            store.clone(),
            shutdown.clone(),
        ));
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
        replay_quarantine(&store).await?;
        store
            .reconcile_pending_reports()
            .await
            .map_err(io::Error::other)?;
        store
            .cleanup_transport_quarantine()
            .await
            .map_err(io::Error::other)?;
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
            if rejected_delivery(ConsumerRole::ApplicationReport, &message.payload).is_some() {
                store
                    .quarantine_transport(ConsumerRole::ApplicationReport, &message.payload)
                    .await
                    .map_err(|error| {
                        metrics::increment("workflow_quarantine_capacity_total");
                        io::Error::other(error)
                    })?;
                metrics::increment("workflow_quarantine_recorded_total");
                message
                    .double_ack()
                    .await
                    .map_err(|_| io::Error::other("workflow quarantine acknowledgement failed"))?;
                metrics::increment("workflow_report_consumer_ack_total");
                continue;
            }
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
                    if !store
                        .retain_ordered_report(&message.payload)
                        .await
                        .map_err(io::Error::other)?
                    {
                        store
                            .accept_terminal_report(&message.payload)
                            .await
                            .map_err(io::Error::other)?;
                    }
                }
                ConversationTurnReportKind::Unspecified => {
                    return Err(io::Error::other("invalid executor report"));
                }
            }
            message
                .double_ack()
                .await
                .map_err(|_| io::Error::other("workflow report acknowledgement failed"))?;
            metrics::increment("workflow_report_consumer_ack_total");
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
    if reconcile_decision_acknowledgements(context, store)
        .await
        .is_err()
    {
        metrics::increment("workflow_decision_ack_observation_errors_total");
    }
    let records = store
        .pending_outbox(OUTBOX_BATCH)
        .await
        .map_err(io::Error::other)?;
    let mut count = 0;
    for record in records {
        let mut headers = HeaderMap::new();
        headers.insert("Nats-Msg-Id", format!("{APPLICATION_SOURCE}:{}", record.id));
        metrics::increment("workflow_publish_attempts_total");
        let started = std::time::Instant::now();
        let publication_started = std::time::SystemTime::now();
        let acknowledgement = match publish_exact(context, &record, headers).await {
            Ok(acknowledgement) => acknowledgement,
            Err(error) => {
                metrics::increment("workflow_publish_failures_total");
                let capacity = publish_capacity_error(&error);
                store
                    .mark_outbox_failed(
                        &record.id,
                        &record.event_bytes,
                        if capacity {
                            "broker_capacity"
                        } else {
                            "broker_unavailable"
                        },
                    )
                    .await
                    .map_err(io::Error::other)?;
                if capacity {
                    continue;
                }
                return Err(io::Error::other(
                    "workflow broker publication remains pending",
                ));
            }
        };
        metrics::increment("workflow_broker_storage_ack_total");
        metrics::observe("workflow_broker_storage_ack_seconds", started.elapsed());
        store
            .mark_outbox_publication_at(
                &record.id,
                &record.event_bytes,
                acknowledgement.sequence,
                publication_started,
            )
            .await
            .map_err(io::Error::other)?;
        count += 1;
    }
    Ok(count)
}

async fn replay_quarantine(store: &WorkflowStore) -> Result<(), io::Error> {
    for event in store
        .quarantine_replay_batch()
        .await
        .map_err(io::Error::other)?
    {
        if let Ok((_, _, report)) = decode_report(&event.exact_bytes) {
            if report.kind == ConversationTurnReportKind::Started as i32 {
                store
                    .accept_claim(&event.exact_bytes)
                    .await
                    .map_err(io::Error::other)?;
            } else if !store
                .retain_ordered_report(&event.exact_bytes)
                .await
                .map_err(io::Error::other)?
            {
                store
                    .accept_terminal_report(&event.exact_bytes)
                    .await
                    .map_err(io::Error::other)?;
            }
            store
                .resolve_quarantine(&event)
                .await
                .map_err(io::Error::other)?;
        }
    }
    Ok(())
}

/// Reconcile targeted ACK floors only while their latest broker copy is unexpired.
///
/// # Errors
/// Returns unavailable transport or authoritative database errors.
pub async fn reconcile_decision_acknowledgements(
    context: &jetstream::Context,
    store: &WorkflowStore,
) -> Result<(), io::Error> {
    let mut stream = context
        .get_stream("ORCHESTRATOR_WORKFLOW_DECISION_V1")
        .await
        .map_err(|_| io::Error::other("decision retention observation unavailable"))?;
    let max_age = stream
        .info()
        .await
        .map_err(|_| io::Error::other("decision retention observation unavailable"))?
        .config
        .max_age;
    let margin = std::cmp::min(Duration::from_secs(1), max_age / 2);
    let observed_at = std::time::Instant::now();
    for record in store
        .decision_acknowledgements()
        .await
        .map_err(io::Error::other)?
    {
        // Consumer names derive only from a validated retained UUID, never an
        // arbitrary report string or subject supplied by a publisher.
        if uuid::Uuid::parse_str(&record.executor_instance_id).is_err() {
            metrics::increment("workflow_decision_invalid_target_total");
            continue;
        }
        let name = format!("executor-{}", record.executor_instance_id);
        let mut consumer: jetstream::consumer::PullConsumer = match context
            .get_consumer_from_stream(&name, "ORCHESTRATOR_WORKFLOW_DECISION_V1")
            .await
        {
            Ok(consumer) => consumer,
            Err(_) => continue, // absent/offline consumer keeps exact retry bytes
        };
        let info = consumer
            .info()
            .await
            .map_err(|_| io::Error::other("decision acknowledgement observation unavailable"))?;
        let expected =
            ayni_orchestrator_events::workflow::decision_subject(&record.executor_instance_id)
                .map_err(|_| io::Error::other("invalid retained decision target"))?;
        if info.config.filter_subject != expected
            || info.config.ack_policy != jetstream::consumer::AckPolicy::Explicit
        {
            metrics::increment("workflow_decision_consumer_contract_error_total");
            continue;
        }
        let floor = info.ack_floor.stream_sequence;
        // Expiry can move a broker's contiguous floor without proving the
        // executor retained an old decision. Only a recent, unexpired publication
        // can complete this handoff. Older copies are republished byte-for-byte.
        if max_age.is_zero()
            || record.publication_age.saturating_add(observed_at.elapsed())
                >= max_age.saturating_sub(margin)
        {
            metrics::increment("workflow_decision_expired_ack_observation_total");
            continue;
        }
        if floor >= record.stream_sequence {
            store
                .acknowledge_decision_delivery(&record.event_id, record.stream_sequence, floor)
                .await
                .map_err(io::Error::other)?;
            metrics::increment("workflow_decision_consumer_ack_total");
        }
    }
    Ok(())
}

async fn observe_broker_pressure(context: &jetstream::Context, store: &WorkflowStore) {
    for (lane, stream_name) in [
        ("command", "ORCHESTRATOR_WORKFLOW_COMMAND_V1"),
        ("report", "ORCHESTRATOR_WORKFLOW_REPORT_V1"),
        ("decision", "ORCHESTRATOR_WORKFLOW_DECISION_V1"),
    ] {
        if let Ok(mut stream) = context.get_stream(stream_name).await
            && let Ok(info) = stream.info().await
        {
            let _ = store
                .observe_broker_capacity(lane, info.state.messages, info.state.bytes)
                .await;
            metrics::gauge("workflow_stream_messages", lane, info.state.messages);
            metrics::gauge("workflow_stream_bytes", lane, info.state.bytes);
            metrics::gauge(
                "workflow_stream_max_messages",
                lane,
                info.config.max_messages.cast_unsigned(),
            );
            metrics::gauge(
                "workflow_stream_max_bytes",
                lane,
                info.config.max_bytes.cast_unsigned(),
            );
        }
    }
    for (lane, stream, name) in [
        (
            "command",
            "ORCHESTRATOR_WORKFLOW_COMMAND_V1",
            "scripted-executor",
        ),
        ("report", REPORT_STREAM, REPORT_CONSUMER),
    ] {
        let consumer: Result<jetstream::consumer::PullConsumer, _> =
            context.get_consumer_from_stream(name, stream).await;
        if let Ok(mut consumer) = consumer
            && let Ok(info) = consumer.info().await
        {
            metrics::gauge("workflow_consumer_pending", lane, info.num_pending);
            metrics::gauge(
                "workflow_consumer_ack_pending",
                lane,
                info.num_ack_pending as u64,
            );
        }
    }
}

async fn publish_exact(
    context: &jetstream::Context,
    record: &ayni_orchestrator_workflow_postgres::model::OutboxRecord,
    headers: HeaderMap,
) -> Result<jetstream::publish::PublishAck, jetstream::context::PublishError> {
    context
        .publish_with_headers(
            record.subject.clone(),
            headers,
            record.event_bytes.clone().into(),
        )
        .await?
        .await
}

fn publish_capacity_error(error: &jetstream::context::PublishError) -> bool {
    error
        .source()
        .and_then(|source| source.downcast_ref::<jetstream::Error>())
        .is_some_and(|error| {
            use jetstream::ErrorCode;
            [
                ErrorCode::STREAM_LIMITS,
                ErrorCode::STORAGE_RESOURCES_EXCEEDED,
                ErrorCode::ACCOUNT_RESOURCES_EXCEEDED,
                ErrorCode::MEMORY_RESOURCES_EXCEEDED,
                ErrorCode::STREAM_MAX_STREAM_BYTES_EXCEEDED,
            ]
            .contains(&error.error_code())
        })
}

async fn record_connection_failure(store: &WorkflowStore) {
    if let Ok(records) = store.pending_outbox(OUTBOX_BATCH).await {
        for record in records {
            if store
                .mark_outbox_failed(&record.id, &record.event_bytes, "broker_unavailable")
                .await
                .is_err()
            {
                metrics::increment("workflow_delivery_observation_errors_total");
            }
        }
    }
}

async fn run_broker_observer(
    context: jetstream::Context,
    store: WorkflowStore,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut tick = time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _=tick.tick() => observe_broker_pressure(&context,&store).await,
            changed=shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { return; } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capacity_failures_are_distinct_from_an_unavailable_broker()
    -> Result<(), Box<dyn std::error::Error>> {
        let capacity: jetstream::Error =
            serde_json::from_str(r#"{"code":503,"err_code":10053,"description":"stream limit"}"#)?;
        let error = jetstream::context::PublishError::with_source(
            jetstream::context::PublishErrorKind::Other,
            capacity,
        );
        assert!(publish_capacity_error(&error));
        assert!(!publish_capacity_error(
            &jetstream::context::PublishError::new(jetstream::context::PublishErrorKind::TimedOut)
        ));
        Ok(())
    }
}
