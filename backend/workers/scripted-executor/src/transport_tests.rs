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

const COMMAND_STREAM: &str = "ORCHESTRATOR_WORKFLOW_COMMAND_V1";
const REPORT_STREAM: &str = "ORCHESTRATOR_WORKFLOW_REPORT_V1";
const DECISION_STREAM: &str = "ORCHESTRATOR_WORKFLOW_DECISION_V1";
const COMMAND_SUBJECT: &str = "orchestrator.workflow.command.conversation.v1";
const REPORT_SUBJECT: &str = "orchestrator.workflow.report.conversation.v1";
const WINNER_CONSUMER: &str = "executor-018f0f7b-6a4d-7a11-8d77-010203040506";
const LOSER_CONSUMER: &str = "executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6";

struct TransportHarness {
    application: jetstream::Context,
    administrator: jetstream::Context,
    winner_transport: jetstream::Context,
    loser_transport: jetstream::Context,
    command_consumer: PullConsumer,
    report_consumer: PullConsumer,
    winner_decisions: PullConsumer,
    loser_decisions: PullConsumer,
}

struct OwnershipScenario {
    winner_directory: TempDir,
    loser_directory: TempDir,
    command: CommandFixture,
    winner_claim: ReportFixture,
    loser_claim: ReportFixture,
}

struct ClaimDecisions {
    winner: Vec<u8>,
    loser: Vec<u8>,
}

async fn transport_harness() -> TransportResult<TransportHarness> {
    let application = transport_context("M1_APPLICATION").await?;
    let administrator = transport_context("M1_ADMIN").await?;
    let winner_transport = transport_context("M1_WINNER").await?;
    let loser_transport = transport_context("M1_LOSER").await?;
    let command_consumer =
        pull_consumer(&winner_transport, COMMAND_STREAM, "scripted-executor").await?;
    let report_consumer =
        pull_consumer(&application, REPORT_STREAM, "workflow-application").await?;
    let winner_decisions =
        pull_consumer(&winner_transport, DECISION_STREAM, WINNER_CONSUMER).await?;
    let loser_decisions = pull_consumer(&loser_transport, DECISION_STREAM, LOSER_CONSUMER).await?;
    Ok(TransportHarness {
        application,
        administrator,
        winner_transport,
        loser_transport,
        command_consumer,
        report_consumer,
        winner_decisions,
        loser_decisions,
    })
}

fn ownership_scenario() -> TransportResult<OwnershipScenario> {
    let command = command_fixture(1, COMMAND_ID)?;
    let winner_claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    let loser_claim = claim_fixture(
        &command,
        LOSER,
        &LOSER_SEED,
        "018f0f7b-6a4d-7a11-8d77-c1c2c3c4c5c6",
    )?;
    Ok(OwnershipScenario {
        winner_directory: TempDir::new()?,
        loser_directory: TempDir::new()?,
        command,
        winner_claim,
        loser_claim,
    })
}

async fn retain_redelivered_claims_and_publish(
    transport: &TransportHarness,
    scenario: &OwnershipScenario,
) -> TransportResult {
    publish_exact(
        &transport.application,
        COMMAND_SUBJECT,
        COMMAND_ID,
        &scenario.command.bytes,
    )
    .await?;
    let first_delivery = next_message(&transport.command_consumer).await?;
    assert_eq!(
        first_delivery.payload.as_ref(),
        scenario.command.bytes.as_slice()
    );
    {
        let mut winner = open(
            &scenario.winner_directory,
            LedgerLimits::m1_default(),
            WINNER,
            &WINNER_SEED,
        )?;
        winner.retain_command_and_claim(
            &scenario.command.command(),
            &scenario.winner_claim.report(1),
        )?;
    }
    drop(first_delivery);

    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let redelivery = next_message(&transport.command_consumer).await?;
    assert_eq!(
        redelivery.payload.as_ref(),
        scenario.command.bytes.as_slice()
    );
    let mut loser = open(
        &scenario.loser_directory,
        LedgerLimits::m1_default(),
        LOSER,
        &LOSER_SEED,
    )?;
    loser.retain_command_and_claim(&scenario.command.command(), &scenario.loser_claim.report(1))?;
    redelivery.double_ack().await?;

    let mut winner = open(
        &scenario.winner_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    assert_eq!(
        winner.retain_redelivered_command(&scenario.command.command())?,
        StoreOutcome::Duplicate
    );
    publish_exact(
        &transport.winner_transport,
        REPORT_SUBJECT,
        &scenario.winner_claim.identity.id,
        &scenario.winner_claim.bytes,
    )
    .await?;
    publish_exact(
        &transport.loser_transport,
        REPORT_SUBJECT,
        &scenario.loser_claim.identity.id,
        &scenario.loser_claim.bytes,
    )
    .await?;
    Ok(())
}

fn claim_decisions(scenario: &OwnershipScenario) -> TransportResult<ClaimDecisions> {
    Ok(ClaimDecisions {
        winner: decision_bytes(
            &scenario.command,
            &scenario.winner_claim,
            WINNER,
            WINNER,
            true,
            None,
            None,
        )?,
        loser: decision_bytes(
            &scenario.command,
            &scenario.loser_claim,
            LOSER,
            WINNER,
            false,
            None,
            None,
        )?,
    })
}

async fn route_claim_decisions(
    transport: &TransportHarness,
    scenario: &OwnershipScenario,
    decisions: &ClaimDecisions,
) -> TransportResult {
    for _ in 0..2 {
        let report = next_message(&transport.report_consumer).await?;
        let (target, report_id, decision) =
            if report.payload.as_ref() == scenario.winner_claim.bytes.as_slice() {
                (
                    WINNER,
                    scenario.winner_claim.identity.id.as_str(),
                    decisions.winner.as_slice(),
                )
            } else if report.payload.as_ref() == scenario.loser_claim.bytes.as_slice() {
                (
                    LOSER,
                    scenario.loser_claim.identity.id.as_str(),
                    decisions.loser.as_slice(),
                )
            } else {
                return Err(io::Error::other("unexpected ownership claim bytes").into());
            };
        publish_exact(
            &transport.application,
            &format!("orchestrator.workflow.accepted.conversation.v1.{target}"),
            &format!("decision:{report_id}"),
            decision,
        )
        .await?;
        report.double_ack().await?;
    }
    Ok(())
}

async fn reconcile_claim_decisions(
    transport: &mut TransportHarness,
    scenario: &OwnershipScenario,
    decisions: &ClaimDecisions,
) -> TransportResult {
    let mut winner = open(
        &scenario.winner_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    let winner_delivery = next_message(&transport.winner_decisions).await?;
    assert_eq!(
        winner_delivery.payload.as_ref(),
        decisions.winner.as_slice()
    );
    let winner_delivery_sequence = winner_delivery.info()?.consumer_sequence;
    winner.record_decision_delivery(
        &Decision {
            exact_bytes: &decisions.winner,
        },
        winner_delivery_sequence,
    )?;
    assert!(winner.can_execute(
        &scenario.command.identity.source,
        &scenario.command.identity.id,
    )?);
    drop(winner_delivery);

    let mut loser = open(
        &scenario.loser_directory,
        LedgerLimits::m1_default(),
        LOSER,
        &LOSER_SEED,
    )?;
    let loser_delivery = next_message(&transport.loser_decisions).await?;
    assert_eq!(loser_delivery.payload.as_ref(), decisions.loser.as_slice());
    let loser_delivery_sequence = loser_delivery.info()?.consumer_sequence;
    loser.record_decision_delivery(
        &Decision {
            exact_bytes: &decisions.loser,
        },
        loser_delivery_sequence,
    )?;
    loser_delivery.double_ack().await?;
    let loser_ack_floor = transport
        .loser_decisions
        .info()
        .await?
        .ack_floor
        .consumer_sequence;
    loser.reconcile_decision_ack_floor(
        &scenario.loser_claim.identity.source,
        &scenario.loser_claim.identity.id,
        LOSER_CONSUMER,
        loser_ack_floor,
    )?;
    assert!(!loser.can_execute(
        &scenario.command.identity.source,
        &scenario.command.identity.id,
    )?);
    assert!(loser.can_retire()?);
    drop(winner);
    Ok(())
}

async fn reconcile_winner_redelivery(
    transport: &mut TransportHarness,
    scenario: &OwnershipScenario,
    decisions: &ClaimDecisions,
) -> TransportResult {
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let repeated_decision = next_message(&transport.winner_decisions).await?;
    let mut winner = open(
        &scenario.winner_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    let repeated_sequence = repeated_decision.info()?.consumer_sequence;
    assert_eq!(
        winner.record_decision_delivery(
            &Decision {
                exact_bytes: &decisions.winner,
            },
            repeated_sequence,
        )?,
        StoreOutcome::Duplicate
    );
    assert!(matches!(
        winner.reconcile_decision_ack_floor(
            &scenario.winner_claim.identity.source,
            &scenario.winner_claim.identity.id,
            LOSER_CONSUMER,
            repeated_sequence,
        ),
        Err(LedgerError::Invalid)
    ));
    assert!(matches!(
        winner.reconcile_decision_ack_floor(
            &scenario.winner_claim.identity.source,
            &scenario.winner_claim.identity.id,
            WINNER_CONSUMER,
            repeated_sequence - 1,
        ),
        Err(LedgerError::DecisionAckPending)
    ));
    repeated_decision.double_ack().await?;
    drop(winner);

    let winner = open(
        &scenario.winner_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    let winner_ack_floor = transport
        .winner_decisions
        .info()
        .await?
        .ack_floor
        .consumer_sequence;
    winner.reconcile_decision_ack_floor(
        &scenario.winner_claim.identity.source,
        &scenario.winner_claim.identity.id,
        WINNER_CONSUMER,
        winner_ack_floor,
    )?;
    Ok(())
}

async fn prove_terminal_republish_and_retirement(
    transport: &mut TransportHarness,
    scenario: &OwnershipScenario,
) -> TransportResult {
    let mut winner = open(
        &scenario.winner_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    let terminal = terminal_fixture(&scenario.command, WINNER, &WINNER_SEED, TERMINAL_ID)?;
    winner.retain_terminal_report(
        &scenario.command.identity.source,
        &scenario.command.identity.id,
        &terminal.report(2),
    )?;
    let first_terminal_ack = publish_exact(
        &transport.winner_transport,
        REPORT_SUBJECT,
        &terminal.identity.id,
        &terminal.bytes,
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let mut report_stream = transport.administrator.get_stream(REPORT_STREAM).await?;
    assert_eq!(report_stream.info().await?.state.messages, 0);

    let replay_ack = publish_exact(
        &transport.winner_transport,
        REPORT_SUBJECT,
        &terminal.identity.id,
        &terminal.bytes,
    )
    .await?;
    assert!(!replay_ack.duplicate);
    assert!(replay_ack.sequence > first_terminal_ack.sequence);
    let duplicate_ack = publish_exact(
        &transport.winner_transport,
        REPORT_SUBJECT,
        &terminal.identity.id,
        &terminal.bytes,
    )
    .await?;
    assert!(duplicate_ack.duplicate);
    assert_eq!(duplicate_ack.sequence, replay_ack.sequence);

    let terminal_delivery = next_message(&transport.report_consumer).await?;
    assert_eq!(
        terminal_delivery.payload.as_ref(),
        terminal.bytes.as_slice()
    );
    let terminal_decision = decision_bytes(
        &scenario.command,
        &terminal,
        WINNER,
        WINNER,
        true,
        None,
        None,
    )?;
    publish_exact(
        &transport.application,
        &format!("orchestrator.workflow.accepted.conversation.v1.{WINNER}"),
        &format!("decision:{}", terminal.identity.id),
        &terminal_decision,
    )
    .await?;
    terminal_delivery.double_ack().await?;

    let terminal_decision_delivery = next_message(&transport.winner_decisions).await?;
    assert_eq!(
        terminal_decision_delivery.payload.as_ref(),
        terminal_decision.as_slice()
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
        &scenario.winner_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    let terminal_ack_floor = transport
        .winner_decisions
        .info()
        .await?
        .ack_floor
        .consumer_sequence;
    winner.reconcile_decision_ack_floor(
        &terminal.identity.source,
        &terminal.identity.id,
        WINNER_CONSUMER,
        terminal_ack_floor,
    )?;
    assert!(winner.can_retire()?);
    Ok(())
}

#[tokio::test]
#[ignore = "requires the disposable authenticated JetStream wrapper"]
async fn authenticated_jetstream_redelivery_preserves_ledger_ownership() -> TransportResult {
    let mut transport = transport_harness().await?;
    let scenario = ownership_scenario()?;
    retain_redelivered_claims_and_publish(&transport, &scenario).await?;
    let decisions = claim_decisions(&scenario)?;
    route_claim_decisions(&transport, &scenario, &decisions).await?;
    reconcile_claim_decisions(&mut transport, &scenario, &decisions).await?;
    reconcile_winner_redelivery(&mut transport, &scenario, &decisions).await?;
    prove_terminal_republish_and_retirement(&mut transport, &scenario).await?;
    println!(
        "executor-transport-feasibility: command-redelivery=pass split-brain=pass decision-redelivery=pass ack-reconciliation=pass long-outage-republish=pass retirement=pass"
    );
    Ok(())
}
