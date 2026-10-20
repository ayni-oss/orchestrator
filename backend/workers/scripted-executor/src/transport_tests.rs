use super::tests::*;
use async_nats::{
    HeaderMap,
    jetstream::{self, consumer::PullConsumer},
};
use std::{env, error::Error, io, time::Duration};
use tokio_stream::StreamExt as _;

type TransportResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

async fn transport_context(prefix: &str) -> TransportResult<jetstream::Context> {
    let endpoint = env::var(format!("{prefix}_NATS_URL"))?;
    let user = env::var(format!("{prefix}_NATS_USER"))?;
    let password = env::var(format!("{prefix}_NATS_PASSWORD"))?;
    let client = async_nats::ConnectOptions::with_user_and_password(user, password)
        .connection_timeout(Duration::from_secs(2))
        .max_reconnects(1)
        .connect(endpoint)
        .await?;
    let mut context = jetstream::new(client);
    context.set_timeout(Duration::from_secs(2));
    Ok(context)
}

async fn pull_consumer(
    context: &jetstream::Context,
    stream: &str,
    consumer: &str,
) -> TransportResult<PullConsumer> {
    Ok(context.get_consumer_from_stream(consumer, stream).await?)
}

async fn next_message(consumer: &PullConsumer) -> TransportResult<jetstream::Message> {
    let mut messages = consumer
        .fetch()
        .max_messages(1)
        .expires(Duration::from_secs(3))
        .messages()
        .await?;
    messages
        .next()
        .await
        .ok_or_else(|| io::Error::other("expected a JetStream delivery"))?
}

async fn publish_exact(
    context: &jetstream::Context,
    subject: &str,
    message_id: &str,
    bytes: &[u8],
) -> TransportResult<jetstream::publish::PublishAck> {
    let mut headers = HeaderMap::new();
    headers.insert("Nats-Msg-Id", message_id);
    Ok(context
        .publish_with_headers(subject.to_owned(), headers, bytes.to_vec().into())
        .await?
        .await?)
}

#[tokio::test]
#[ignore = "requires the disposable authenticated JetStream wrapper"]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end transport scenario owns crash, split-brain, replay, and retirement state"
)]
async fn authenticated_jetstream_redelivery_preserves_ledger_ownership() -> TransportResult {
    const COMMAND_STREAM: &str = "ORCHESTRATOR_WORKFLOW_COMMAND_V1";
    const REPORT_STREAM: &str = "ORCHESTRATOR_WORKFLOW_REPORT_V1";
    const DECISION_STREAM: &str = "ORCHESTRATOR_WORKFLOW_DECISION_V1";
    const COMMAND_SUBJECT: &str = "orchestrator.workflow.command.conversation.v1";
    const REPORT_SUBJECT: &str = "orchestrator.workflow.report.conversation.v1";
    const WINNER_CONSUMER: &str = "executor-018f0f7b-6a4d-7a11-8d77-010203040506";
    const LOSER_CONSUMER: &str = "executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6";

    let application = transport_context("M1_APPLICATION").await?;
    let administrator = transport_context("M1_ADMIN").await?;
    let winner_transport = transport_context("M1_WINNER").await?;
    let loser_transport = transport_context("M1_LOSER").await?;
    let command_consumer =
        pull_consumer(&winner_transport, COMMAND_STREAM, "scripted-executor").await?;
    let report_consumer =
        pull_consumer(&application, REPORT_STREAM, "workflow-application").await?;
    let mut winner_decisions =
        pull_consumer(&winner_transport, DECISION_STREAM, WINNER_CONSUMER).await?;
    let mut loser_decisions =
        pull_consumer(&loser_transport, DECISION_STREAM, LOSER_CONSUMER).await?;

    let winner_directory = TempDir::new()?;
    let loser_directory = TempDir::new()?;
    let command = command_fixture(1, COMMAND_ID)?;
    let winner_claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    let loser_claim = claim_fixture(
        &command,
        LOSER,
        &LOSER_SEED,
        "018f0f7b-6a4d-7a11-8d77-c1c2c3c4c5c6",
    )?;

    publish_exact(&application, COMMAND_SUBJECT, COMMAND_ID, &command.bytes).await?;
    let first_delivery = next_message(&command_consumer).await?;
    assert_eq!(first_delivery.payload.as_ref(), command.bytes);
    {
        let mut winner = open(
            &winner_directory,
            LedgerLimits::m1_default(),
            WINNER,
            &WINNER_SEED,
        )?;
        winner.retain_command_and_claim(&command.command(), &winner_claim.report(1))?;
    }
    drop(first_delivery);

    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let redelivery = next_message(&command_consumer).await?;
    assert_eq!(redelivery.payload.as_ref(), command.bytes);
    let mut loser = open(
        &loser_directory,
        LedgerLimits::m1_default(),
        LOSER,
        &LOSER_SEED,
    )?;
    loser.retain_command_and_claim(&command.command(), &loser_claim.report(1))?;
    redelivery.double_ack().await?;

    let mut winner = open(
        &winner_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    assert_eq!(
        winner.retain_redelivered_command(&command.command())?,
        StoreOutcome::Duplicate
    );
    publish_exact(
        &winner_transport,
        REPORT_SUBJECT,
        &winner_claim.identity.id,
        &winner_claim.bytes,
    )
    .await?;
    publish_exact(
        &loser_transport,
        REPORT_SUBJECT,
        &loser_claim.identity.id,
        &loser_claim.bytes,
    )
    .await?;

    let winner_decision =
        decision_bytes(&command, &winner_claim, WINNER, WINNER, true, None, None)?;
    let loser_decision = decision_bytes(&command, &loser_claim, LOSER, WINNER, false, None, None)?;
    for _ in 0..2 {
        let report = next_message(&report_consumer).await?;
        let (target, report_id, decision) = if report.payload.as_ref() == winner_claim.bytes {
            (
                WINNER,
                winner_claim.identity.id.as_str(),
                winner_decision.as_slice(),
            )
        } else if report.payload.as_ref() == loser_claim.bytes {
            (
                LOSER,
                loser_claim.identity.id.as_str(),
                loser_decision.as_slice(),
            )
        } else {
            return Err(io::Error::other("unexpected ownership claim bytes").into());
        };
        publish_exact(
            &application,
            &format!("orchestrator.workflow.accepted.conversation.v1.{target}"),
            &format!("decision:{report_id}"),
            decision,
        )
        .await?;
        report.double_ack().await?;
    }

    let winner_delivery = next_message(&winner_decisions).await?;
    assert_eq!(winner_delivery.payload.as_ref(), winner_decision);
    let winner_delivery_sequence = winner_delivery.info()?.consumer_sequence;
    winner.record_decision_delivery(
        &Decision {
            exact_bytes: &winner_decision,
        },
        winner_delivery_sequence,
    )?;
    assert!(winner.can_execute(&command.identity.source, &command.identity.id)?);
    drop(winner_delivery);

    let loser_delivery = next_message(&loser_decisions).await?;
    assert_eq!(loser_delivery.payload.as_ref(), loser_decision);
    let loser_delivery_sequence = loser_delivery.info()?.consumer_sequence;
    loser.record_decision_delivery(
        &Decision {
            exact_bytes: &loser_decision,
        },
        loser_delivery_sequence,
    )?;
    loser_delivery.double_ack().await?;
    let loser_ack_floor = loser_decisions.info().await?.ack_floor.consumer_sequence;
    loser.reconcile_decision_ack_floor(
        &loser_claim.identity.source,
        &loser_claim.identity.id,
        LOSER_CONSUMER,
        loser_ack_floor,
    )?;
    assert!(!loser.can_execute(&command.identity.source, &command.identity.id)?);
    assert!(loser.can_retire()?);

    drop(winner);
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let repeated_decision = next_message(&winner_decisions).await?;
    let mut winner = open(
        &winner_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    let repeated_sequence = repeated_decision.info()?.consumer_sequence;
    assert_eq!(
        winner.record_decision_delivery(
            &Decision {
                exact_bytes: &winner_decision,
            },
            repeated_sequence,
        )?,
        StoreOutcome::Duplicate
    );
    assert!(matches!(
        winner.reconcile_decision_ack_floor(
            &winner_claim.identity.source,
            &winner_claim.identity.id,
            LOSER_CONSUMER,
            repeated_sequence,
        ),
        Err(LedgerError::Invalid)
    ));
    assert!(matches!(
        winner.reconcile_decision_ack_floor(
            &winner_claim.identity.source,
            &winner_claim.identity.id,
            WINNER_CONSUMER,
            repeated_sequence - 1,
        ),
        Err(LedgerError::DecisionAckPending)
    ));
    repeated_decision.double_ack().await?;
    drop(winner);
    let mut winner = open(
        &winner_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    let winner_ack_floor = winner_decisions.info().await?.ack_floor.consumer_sequence;
    winner.reconcile_decision_ack_floor(
        &winner_claim.identity.source,
        &winner_claim.identity.id,
        WINNER_CONSUMER,
        winner_ack_floor,
    )?;

    let terminal = terminal_fixture(&command, WINNER, &WINNER_SEED, TERMINAL_ID)?;
    winner.retain_terminal_report(
        &command.identity.source,
        &command.identity.id,
        &terminal.report(2),
    )?;
    let first_terminal_ack = publish_exact(
        &winner_transport,
        REPORT_SUBJECT,
        &terminal.identity.id,
        &terminal.bytes,
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let mut report_stream = administrator.get_stream(REPORT_STREAM).await?;
    assert_eq!(report_stream.info().await?.state.messages, 0);

    let replay_ack = publish_exact(
        &winner_transport,
        REPORT_SUBJECT,
        &terminal.identity.id,
        &terminal.bytes,
    )
    .await?;
    assert!(!replay_ack.duplicate);
    assert!(replay_ack.sequence > first_terminal_ack.sequence);
    let duplicate_ack = publish_exact(
        &winner_transport,
        REPORT_SUBJECT,
        &terminal.identity.id,
        &terminal.bytes,
    )
    .await?;
    assert!(duplicate_ack.duplicate);
    assert_eq!(duplicate_ack.sequence, replay_ack.sequence);

    let terminal_delivery = next_message(&report_consumer).await?;
    assert_eq!(terminal_delivery.payload.as_ref(), terminal.bytes);
    let terminal_decision = decision_bytes(&command, &terminal, WINNER, WINNER, true, None, None)?;
    publish_exact(
        &application,
        &format!("orchestrator.workflow.accepted.conversation.v1.{WINNER}"),
        &format!("decision:{}", terminal.identity.id),
        &terminal_decision,
    )
    .await?;
    terminal_delivery.double_ack().await?;

    let terminal_decision_delivery = next_message(&winner_decisions).await?;
    assert_eq!(
        terminal_decision_delivery.payload.as_ref(),
        terminal_decision
    );
    let terminal_decision_sequence = terminal_decision_delivery.info()?.consumer_sequence;
    winner.record_decision_delivery(
        &Decision {
            exact_bytes: &terminal_decision,
        },
        terminal_decision_sequence,
    )?;
    terminal_decision_delivery.double_ack().await?;
    drop(winner);
    let winner = open(
        &winner_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    let terminal_ack_floor = winner_decisions.info().await?.ack_floor.consumer_sequence;
    winner.reconcile_decision_ack_floor(
        &terminal.identity.source,
        &terminal.identity.id,
        WINNER_CONSUMER,
        terminal_ack_floor,
    )?;
    assert!(winner.can_retire()?);
    println!(
        "executor-transport-feasibility: command-redelivery=pass split-brain=pass decision-redelivery=pass ack-reconciliation=pass long-outage-republish=pass retirement=pass"
    );
    Ok(())
}
