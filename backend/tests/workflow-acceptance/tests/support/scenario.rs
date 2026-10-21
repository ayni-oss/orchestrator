use crate::{
    Result,
    fixture::{self, CommandFixture, Runtime, Scope, SignedReport},
    transport::{self, LOSER, LOSER_CONSUMER, Transport, WINNER, WINNER_CONSUMER},
};
use ayni_orchestrator_api::v1::{self, watch_conversation_response};
use ayni_orchestrator_events::v1::ConversationTurnReportKind as Kind;
use ayni_orchestrator_scripted_executor::{Decision, LedgerError, StoreOutcome};
use ayni_orchestrator_workflow_postgres::model::{OutboxRecord, ReportDecisionReceipt};
use std::{collections::BTreeMap, time::Duration};
use tempfile::TempDir;
use uuid::Uuid;

#[derive(Default)]
struct Timeline {
    cases: BTreeMap<&'static str, u64>,
    ordinal: u64,
}
impl Timeline {
    fn passed(&mut self, boundary: &'static str) {
        self.ordinal += 1;
        *self.cases.entry(boundary).or_default() += 1;
        println!(
            "{}",
            serde_json::json!({"fixture":"canonical-pg-jetstream-ledger","boundary":boundary,"ordinal":self.ordinal,"outcome":"pass"})
        );
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the cumulative acceptance narrative keeps its ordered durability boundaries visible"
)]
pub async fn run() -> Result {
    let mut runtime = Runtime::start().await?;
    let mut transport = Transport::connect().await?;
    let scope = Scope::create(&mut runtime).await?;
    let key = Uuid::now_v7().to_string();
    let a = scope.send(&mut runtime, &key, "input A").await?;
    assert_eq!(scope.send(&mut runtime, &key, "input A").await?, a);
    let b = scope
        .send(&mut runtime, &Uuid::now_v7().to_string(), "input B")
        .await?;
    let c = scope
        .send(&mut runtime, &Uuid::now_v7().to_string(), "input C")
        .await?;
    let initial = scope.snapshot(&mut runtime).await?;
    assert_eq!(initial.nonterminal_turns.len(), 3);
    assert_eq!(
        initial
            .nonterminal_turns
            .iter()
            .filter(|turn| turn.scheduling_state == v1::SchedulingState::Eligible as i32)
            .count(),
        1
    );
    let winner = TempDir::new()?;
    let loser = TempDir::new()?;
    let mut timeline = Timeline::default();
    let command = CommandFixture::decode(runtime.command_record(&a.turn_id).await?)?;
    assert_eq!(command.payload.input_content, "input A");
    assert!(runtime.command_record(&b.turn_id).await.is_err());
    runtime.restart().await?;
    assert_eq!(
        runtime.command_record(&a.turn_id).await?.event_bytes,
        command.record.event_bytes
    );
    timeline.passed("command_commit_before_publication_restart");
    command_faults(
        &runtime,
        &mut transport,
        &command,
        &winner,
        &loser,
        &mut timeline,
    )
    .await?;
    let winner_claim =
        SignedReport::new(&command, WINNER, &fixture::WINNER_SEED, Kind::Started, "")?;
    let loser_claim = SignedReport::new(&command, LOSER, &fixture::LOSER_SEED, Kind::Started, "")?;
    // Existing exact claims were created before command ACK. Recover them from
    // each ledger rather than constructing a new report identity on restart.
    let winner_claim = recovered_claim(&winner, WINNER, &fixture::WINNER_SEED, winner_claim)?;
    let loser_claim = recovered_claim(&loser, LOSER, &fixture::LOSER_SEED, loser_claim)?;
    let (accepted, rejected) = claim_faults(
        &mut runtime,
        &transport,
        &winner_claim,
        &loser_claim,
        &mut timeline,
    )
    .await?;
    decision_loss_and_recovery(
        &mut runtime,
        &mut transport,
        &command,
        &winner,
        &loser,
        &winner_claim,
        &loser_claim,
        &accepted,
        &rejected,
        &mut timeline,
    )
    .await?;
    fail_predecessor(
        &mut runtime,
        &mut transport,
        &scope,
        &command,
        &winner,
        &mut timeline,
    )
    .await?;
    let b_command = CommandFixture::decode(runtime.command_record(&b.turn_id).await?)?;
    assert_eq!(context(&b_command), ["input A", "input B"]);
    assert!(!context(&b_command).contains(&"input C"));
    timeline.passed("failed_predecessor_promotes_only_B_with_frozen_context");
    complete_successor(
        &mut runtime,
        &mut transport,
        &scope,
        &b_command,
        &winner,
        "response B",
        true,
        &mut timeline,
    )
    .await?;
    assert_eq!(
        runtime.command_record(&b.turn_id).await?.event_bytes,
        b_command.record.event_bytes
    );
    let c_command = CommandFixture::decode(runtime.command_record(&c.turn_id).await?)?;
    assert_eq!(
        context(&c_command),
        ["input A", "input B", "response B", "input C"]
    );
    timeline.passed("C_freezes_only_after_B_terminal_commit");
    complete_successor(
        &mut runtime,
        &mut transport,
        &scope,
        &c_command,
        &winner,
        "response C",
        false,
        &mut timeline,
    )
    .await?;
    let final_snapshot = scope.snapshot(&mut runtime).await?;
    assert!(final_snapshot.nonterminal_turns.is_empty());
    assert_eq!(final_snapshot.recent_terminal_turns.len(), 3);
    assert_eq!(
        final_snapshot
            .recent_messages
            .iter()
            .filter(|message| message.role == v1::MessageRole::Assistant as i32)
            .count(),
        2
    );
    assert_eq!(
        final_snapshot
            .recent_terminal_turns
            .iter()
            .filter(|turn| turn.presentation_state == v1::PresentationState::Failed as i32)
            .count(),
        1
    );
    replay_every_revision(&mut runtime, &scope, &initial, &final_snapshot).await?;
    timeline.passed("canonical_response_and_cursor_reconnect_exactly_once");
    assert!(fixture::ledger(&winner, WINNER, &fixture::WINNER_SEED)?.can_retire()?);
    assert!(fixture::ledger(&loser, LOSER, &fixture::LOSER_SEED)?.can_retire()?);
    println!(
        "{}",
        serde_json::json!({"fixture":"canonical-pg-jetstream-ledger","outcome":"pass","cases":timeline.cases,"turns":3,"assistant_responses":2,"observed_revision":final_snapshot.conversation_revision})
    );
    Ok(())
}

fn context(command: &CommandFixture) -> Vec<&str> {
    command
        .payload
        .context
        .iter()
        .map(|record| record.content.as_str())
        .collect()
}

async fn publish_record(
    runtime: &Runtime,
    transport: &Transport,
    record: &OutboxRecord,
) -> Result<u64> {
    let publication_started = std::time::SystemTime::now();
    let ack = transport::publish(
        &transport.application,
        &record.subject,
        &record.id,
        &record.event_bytes,
    )
    .await?;
    runtime
        .store
        .mark_outbox_publication_at(
            &record.id,
            &record.event_bytes,
            ack.sequence,
            publication_started,
        )
        .await?;
    Ok(ack.sequence)
}

async fn command_faults(
    runtime: &Runtime,
    transport: &mut Transport,
    command: &CommandFixture,
    winner: &TempDir,
    loser: &TempDir,
    timeline: &mut Timeline,
) -> Result {
    let lost_ack = transport::publish(
        &transport.application,
        &command.record.subject,
        &command.record.id,
        &command.record.event_bytes,
    )
    .await?;
    let repeated = transport::publish(
        &transport.application,
        &command.record.subject,
        &command.record.id,
        &command.record.event_bytes,
    )
    .await?;
    assert!(repeated.duplicate);
    assert_eq!(lost_ack.sequence, repeated.sequence);
    timeline.passed("command_storage_ACK_lost_exact_duplicate");
    transport::restart_fixture("M1_FIXTURE_NATS_CONTAINER").await?;
    *transport = Transport::connect().await?;
    publish_record(runtime, transport, &command.record).await?;
    timeline.passed("broker_restart_republishes_retained_command");
    let delivery = transport::next(&transport.commands).await?;
    assert_eq!(delivery.payload.as_ref(), command.record.event_bytes);
    let claim = SignedReport::new(command, WINNER, &fixture::WINNER_SEED, Kind::Started, "")?;
    fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?
        .retain_command_and_claim(&command.command(), &claim.report())?;
    drop(delivery);
    assert_eq!(
        fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?
            .exact_report_bytes(&claim.identity.source, &claim.identity.id)?,
        claim.bytes
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let redelivery = transport::next(&transport.commands).await?;
    assert_eq!(redelivery.payload.as_ref(), command.record.event_bytes);
    let losing_claim = SignedReport::new(command, LOSER, &fixture::LOSER_SEED, Kind::Started, "")?;
    fixture::ledger(loser, LOSER, &fixture::LOSER_SEED)?
        .retain_command_and_claim(&command.command(), &losing_claim.report())?;
    redelivery.double_ack().await?;
    let mut reopened = fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?;
    assert_eq!(
        reopened.retain_redelivered_command(&command.command())?,
        StoreOutcome::Duplicate
    );
    assert!(!reopened.can_execute(&command.identity.source, &command.identity.id)?);
    timeline.passed("ledger_commit_before_command_ACK_split_brain_redelivery");
    Ok(())
}

fn recovered_claim(
    directory: &TempDir,
    executor: &str,
    seed: &[u8; 32],
    mut fixture: SignedReport,
) -> Result<SignedReport> {
    let events = fixture::ledger(directory, executor, seed)?.pending_reports()?;
    assert_eq!(events.len(), 1);
    let retained = events.into_iter().next().ok_or("claim not retained")?;
    fixture.identity.source = retained.source;
    fixture.identity.id = retained.id;
    fixture.bytes = retained.exact_bytes;
    Ok(fixture)
}

async fn claim_faults(
    runtime: &mut Runtime,
    transport: &Transport,
    winner: &SignedReport,
    loser: &SignedReport,
    timeline: &mut Timeline,
) -> Result<(ReportDecisionReceipt, ReportDecisionReceipt)> {
    transport::publish(
        &transport.winner,
        transport::REPORT_SUBJECT,
        &winner.identity.id,
        &winner.bytes,
    )
    .await?;
    let delivery = transport::next(&transport.reports).await?;
    assert_eq!(delivery.payload.as_ref(), winner.bytes);
    let receipt = runtime.store.accept_claim(&delivery.payload).await?;
    assert!(receipt.accepted);
    drop(delivery);
    runtime.restart().await?;
    assert_eq!(runtime.store.accept_claim(&winner.bytes).await?, receipt);
    transport::publish(
        &transport.loser,
        transport::REPORT_SUBJECT,
        &loser.identity.id,
        &loser.bytes,
    )
    .await?;
    let delivery = transport::next(&transport.reports).await?;
    let rejected = if delivery.payload.as_ref() == loser.bytes {
        let rejected = runtime.store.accept_claim(&delivery.payload).await?;
        delivery.double_ack().await?;
        rejected
    } else {
        assert_eq!(delivery.payload.as_ref(), winner.bytes);
        assert_eq!(
            runtime.store.accept_claim(&delivery.payload).await?,
            receipt
        );
        delivery.double_ack().await?;
        let losing_delivery = transport::next(&transport.reports).await?;
        assert_eq!(losing_delivery.payload.as_ref(), loser.bytes);
        let rejected = runtime.store.accept_claim(&losing_delivery.payload).await?;
        losing_delivery.double_ack().await?;
        rejected
    };
    assert!(!rejected.accepted);
    assert_eq!(rejected.safe_reason, "claim_lost");
    // Replay after the application committed but before it ACKed the broker.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    transport::publish(
        &transport.winner,
        transport::REPORT_SUBJECT,
        &winner.identity.id,
        &winner.bytes,
    )
    .await?;
    let repeated = transport::next(&transport.reports).await?;
    assert_eq!(repeated.payload.as_ref(), winner.bytes);
    assert_eq!(
        runtime.store.accept_claim(&repeated.payload).await?,
        receipt
    );
    repeated.double_ack().await?;
    timeline.passed("claim_commit_before_report_ACK_application_restart");
    timeline.passed("same_generation_competitor_receives_real_rejection");
    Ok((receipt, rejected))
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one explicit crash/recovery narrative retains both competing ledger identities"
)]
async fn decision_loss_and_recovery(
    runtime: &mut Runtime,
    transport: &mut Transport,
    command: &CommandFixture,
    winner: &TempDir,
    loser: &TempDir,
    winning_claim: &SignedReport,
    losing_claim: &SignedReport,
    accepted: &ReportDecisionReceipt,
    rejected: &ReportDecisionReceipt,
    timeline: &mut Timeline,
) -> Result {
    let winning = runtime.decision_record(accepted).await?;
    let losing = runtime.decision_record(rejected).await?;
    runtime.restart().await?;
    assert_eq!(
        runtime.decision_record(accepted).await?.event_bytes,
        winning.event_bytes
    );
    timeline.passed("decision_commit_before_publication_application_restart");
    let original_sequence = publish_record(runtime, transport, &winning).await?;
    publish_record(runtime, transport, &losing).await?;
    let suppressed = transport::next(&transport.winner_decisions).await?;
    assert_eq!(suppressed.payload.as_ref(), winning.event_bytes);
    drop(suppressed);
    prove_expired_decision_retained(runtime, transport, &winning, original_sequence).await?;
    for (directory, executor, seed, claim) in [
        (winner, WINNER, &fixture::WINNER_SEED, winning_claim),
        (loser, LOSER, &fixture::LOSER_SEED, losing_claim),
    ] {
        let ledger = fixture::ledger(directory, executor, seed)?;
        assert!(!ledger.can_execute(&command.identity.source, &command.identity.id)?);
        assert_eq!(
            ledger.exact_report_bytes(&claim.identity.source, &claim.identity.id)?,
            claim.bytes
        );
        assert!(!ledger.can_retire()?);
    }
    assert_eq!(
        runtime.store.accept_claim(&winning_claim.bytes).await?,
        *accepted
    );
    assert_eq!(
        runtime.store.accept_claim(&losing_claim.bytes).await?,
        *rejected
    );
    timeline.passed("suppressed_winner_and_loser_decisions_expire_without_retirement");
    let winner_sequence = publish_record(runtime, transport, &winning).await?;
    assert!(winner_sequence > original_sequence);
    publish_record(runtime, transport, &losing).await?;
    let delivery = transport::next(&transport.winner_decisions).await?;
    assert_eq!(delivery.payload.as_ref(), winning.event_bytes);
    let sequence = delivery.info()?.consumer_sequence;
    fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?.record_decision_delivery(
        &Decision {
            exact_bytes: &delivery.payload,
        },
        sequence,
    )?;
    drop(delivery);
    let reopened = fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?;
    assert!(reopened.can_execute(&command.identity.source, &command.identity.id)?);
    assert!(matches!(
        reopened.reconcile_decision_ack_floor(
            &winning_claim.identity.source,
            &winning_claim.identity.id,
            WINNER_CONSUMER,
            sequence - 1
        ),
        Err(LedgerError::DecisionAckPending)
    ));
    drop(reopened);
    // Resolve the losing target independently; its rejection cannot authorize work.
    record_and_ack(runtime, transport, loser, losing_claim, &losing, true).await?;
    assert!(
        !fixture::ledger(loser, LOSER, &fixture::LOSER_SEED)?
            .can_execute(&command.identity.source, &command.identity.id)?
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let repeated = transport::next(&transport.winner_decisions).await?;
    assert_eq!(repeated.payload.as_ref(), winning.event_bytes);
    let repeated_sequence = repeated.info()?.consumer_sequence;
    assert_eq!(
        fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?.record_decision_delivery(
            &Decision {
                exact_bytes: &repeated.payload
            },
            repeated_sequence
        )?,
        StoreOutcome::Duplicate
    );
    repeated.double_ack().await?;
    let floor = transport.winner_decisions.info().await?.ack_floor;
    fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?.reconcile_decision_ack_floor(
        &winning_claim.identity.source,
        &winning_claim.identity.id,
        WINNER_CONSUMER,
        floor.consumer_sequence,
    )?;
    ayni_orchestrator_control_plane::broker::reconcile_decision_acknowledgements(
        &transport.application,
        &runtime.store,
    )
    .await?;
    if runtime
        .store
        .decision_acknowledgements()
        .await?
        .iter()
        .any(|record| record.event_id == winning.id)
    {
        // Its actual ACK arrived inside the conservative expiry safety margin;
        // a fresh retained copy repairs the observation without changing bytes.
        publish_record(runtime, transport, &winning).await?;
        record_and_ack(runtime, transport, winner, winning_claim, &winning, false).await?;
    }
    assert!(
        !runtime
            .store
            .decision_acknowledgements()
            .await?
            .iter()
            .any(|record| record.event_id == winning.id)
    );
    assert!(winner_sequence > original_sequence);
    timeline.passed("decision_ledger_commit_before_ACK_and_ACK_before_reconciliation_restart");
    Ok(())
}

/// Broker expiry may advance a consumer floor without an ACK. A delayed duplicate
/// storage acknowledgement must preserve the publication's original age, so the
/// production reconciler leaves exact `PostgreSQL` retry evidence authoritative.
async fn prove_expired_decision_retained(
    runtime: &Runtime,
    transport: &mut Transport,
    winning: &OutboxRecord,
    original_sequence: u64,
) -> Result {
    tokio::time::sleep(Duration::from_millis(2300)).await;
    let mut expired_stream = transport
        .administrator
        .get_stream(transport::DECISION_STREAM)
        .await?;
    assert_eq!(expired_stream.info().await?.state.messages, 0);
    let winner_floor = transport
        .winner_decisions
        .info()
        .await?
        .ack_floor
        .stream_sequence;
    // A delayed duplicate storage ACK cannot make this expired copy fresh.
    runtime
        .store
        .mark_outbox_publication_at(
            &winning.id,
            &winning.event_bytes,
            original_sequence,
            std::time::SystemTime::now(),
        )
        .await?;
    let retained = runtime.store.decision_acknowledgements().await?;
    assert!(
        retained.iter().any(|record| record.event_id == winning.id
            && record.publication_age >= Duration::from_secs(2))
    );
    ayni_orchestrator_control_plane::broker::reconcile_decision_acknowledgements(
        &transport.application,
        &runtime.store,
    )
    .await?;
    assert!(
        runtime
            .store
            .decision_acknowledgements()
            .await?
            .iter()
            .any(|record| record.event_id == winning.id)
    );
    println!(
        "{}",
        serde_json::json!({"boundary":"expired_decision_ACK_floor","floor":winner_floor,"expired_publication":original_sequence,"authority_retained":true})
    );
    Ok(())
}

async fn record_and_ack(
    runtime: &Runtime,
    transport: &mut Transport,
    directory: &TempDir,
    report: &SignedReport,
    decision: &OutboxRecord,
    losing: bool,
) -> Result {
    let (consumer, executor, seed, name) = if losing {
        (
            &mut transport.loser_decisions,
            LOSER,
            &fixture::LOSER_SEED,
            LOSER_CONSUMER,
        )
    } else {
        (
            &mut transport.winner_decisions,
            WINNER,
            &fixture::WINNER_SEED,
            WINNER_CONSUMER,
        )
    };
    let delivery = transport::next(consumer).await?;
    assert_eq!(delivery.payload.as_ref(), decision.event_bytes);
    let info = delivery.info()?;
    let stream_sequence = info.stream_sequence;
    let consumer_sequence = info.consumer_sequence;
    fixture::ledger(directory, executor, seed)?.record_decision_delivery(
        &Decision {
            exact_bytes: &delivery.payload,
        },
        consumer_sequence,
    )?;
    delivery.double_ack().await?;
    let floor = consumer.info().await?.ack_floor;
    fixture::ledger(directory, executor, seed)?.reconcile_decision_ack_floor(
        &report.identity.source,
        &report.identity.id,
        name,
        floor.consumer_sequence,
    )?;
    assert!(floor.stream_sequence >= stream_sequence);
    ayni_orchestrator_control_plane::broker::reconcile_decision_acknowledgements(
        &transport.application,
        &runtime.store,
    )
    .await?;
    assert!(
        !runtime
            .store
            .decision_acknowledgements()
            .await?
            .iter()
            .any(|record| record.event_id == decision.id)
    );
    Ok(())
}

async fn accept_terminal(
    runtime: &Runtime,
    transport: &Transport,
    report: &SignedReport,
) -> Result<ReportDecisionReceipt> {
    transport::publish(
        &transport.winner,
        transport::REPORT_SUBJECT,
        &report.identity.id,
        &report.bytes,
    )
    .await?;
    let delivery = transport::next(&transport.reports).await?;
    assert_eq!(delivery.payload.as_ref(), report.bytes);
    let receipt = runtime
        .store
        .accept_terminal_report(&delivery.payload)
        .await?;
    assert!(receipt.accepted);
    delivery.double_ack().await?;
    Ok(receipt)
}

async fn fail_predecessor(
    runtime: &mut Runtime,
    transport: &mut Transport,
    scope: &Scope,
    command: &CommandFixture,
    winner: &TempDir,
    timeline: &mut Timeline,
) -> Result {
    let report = SignedReport::new(command, WINNER, &fixture::WINNER_SEED, Kind::Failed, "")?;
    fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?.retain_terminal_report(
        &command.identity.source,
        &command.identity.id,
        &report.report(),
    )?;
    assert_eq!(
        fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?.pending_reports()?[0].exact_bytes,
        report.bytes
    );
    transport::publish(
        &transport.winner,
        transport::REPORT_SUBJECT,
        &report.identity.id,
        &report.bytes,
    )
    .await?;
    transport::restart_fixture("M1_FIXTURE_NATS_CONTAINER").await?;
    *transport = Transport::connect().await?;
    transport::publish(
        &transport.winner,
        transport::REPORT_SUBJECT,
        &report.identity.id,
        &report.bytes,
    )
    .await?;
    let delivery = transport::next(&transport.reports).await?;
    assert_eq!(delivery.payload.as_ref(), report.bytes);
    let receipt = runtime
        .store
        .accept_terminal_report(&delivery.payload)
        .await?;
    assert!(receipt.accepted);
    drop(delivery);
    transport::restart_fixture("M1_FIXTURE_POSTGRES_CONTAINER").await?;
    runtime.wait_database().await?;
    runtime.restart().await?;
    assert_eq!(
        runtime.store.accept_terminal_report(&report.bytes).await?,
        receipt
    );
    let snapshot = scope.snapshot(runtime).await?;
    assert_eq!(snapshot.recent_terminal_turns.len(), 1);
    assert_eq!(snapshot.nonterminal_turns.len(), 2);
    assert_eq!(
        snapshot.recent_terminal_turns[0].presentation_state,
        v1::PresentationState::Failed as i32
    );
    // A retained terminal result repairs broker loss without rerunning work.
    tokio::time::sleep(Duration::from_millis(2300)).await;
    assert_eq!(accept_terminal(runtime, transport, &report).await?, receipt);
    let decision = runtime.decision_record(&receipt).await?;
    publish_record(runtime, transport, &decision).await?;
    record_and_ack(runtime, transport, winner, &report, &decision, false).await?;
    timeline.passed("terminal_ledger_commit_before_publication_broker_restart");
    timeline.passed("terminal_commit_before_ACK_postgres_and_application_restart");
    timeline.passed("terminal_storage_expiry_replays_exact_bytes_without_execution");
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "canonical scope and retained ledger remain separate authorities in the acceptance fixture"
)]
async fn complete_successor(
    runtime: &mut Runtime,
    transport: &mut Transport,
    scope: &Scope,
    command: &CommandFixture,
    winner: &TempDir,
    response: &str,
    restart_processing: bool,
    timeline: &mut Timeline,
) -> Result {
    publish_record(runtime, transport, &command.record).await?;
    let delivery = transport::next(&transport.commands).await?;
    assert_eq!(delivery.payload.as_ref(), command.record.event_bytes);
    let claim = SignedReport::new(command, WINNER, &fixture::WINNER_SEED, Kind::Started, "")?;
    fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?
        .retain_command_and_claim(&command.command(), &claim.report())?;
    delivery.double_ack().await?;
    transport::publish(
        &transport.winner,
        transport::REPORT_SUBJECT,
        &claim.identity.id,
        &claim.bytes,
    )
    .await?;
    let delivery = transport::next(&transport.reports).await?;
    assert_eq!(delivery.payload.as_ref(), claim.bytes);
    let receipt = runtime.store.accept_claim(&delivery.payload).await?;
    assert!(receipt.accepted);
    delivery.double_ack().await?;
    let decision = runtime.decision_record(&receipt).await?;
    publish_record(runtime, transport, &decision).await?;
    record_and_ack(runtime, transport, winner, &claim, &decision, false).await?;
    if restart_processing {
        runtime.restart().await?;
        let snapshot = scope.snapshot(runtime).await?;
        assert!(
            snapshot
                .nonterminal_turns
                .iter()
                .any(|turn| turn.id == command.binding.turn_id
                    && turn.execution_state == v1::ExecutionState::Processing as i32)
        );
        let executable =
            fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?.executable_commands()?;
        assert_eq!(executable.len(), 1);
        assert_eq!(executable[0].exact_bytes, command.record.event_bytes);
        timeline.passed("processing_restart_recovers_one_accepted_executable_command");
    }
    let terminal = SignedReport::new(
        command,
        WINNER,
        &fixture::WINNER_SEED,
        Kind::Completed,
        response,
    )?;
    fixture::ledger(winner, WINNER, &fixture::WINNER_SEED)?.retain_terminal_report(
        &command.identity.source,
        &command.identity.id,
        &terminal.report(),
    )?;
    let receipt = accept_terminal(runtime, transport, &terminal).await?;
    assert_eq!(
        runtime
            .store
            .accept_terminal_report(&terminal.bytes)
            .await?,
        receipt
    );
    let decision = runtime.decision_record(&receipt).await?;
    publish_record(runtime, transport, &decision).await?;
    record_and_ack(runtime, transport, winner, &terminal, &decision, false).await?;
    timeline.passed("terminal_response_duplicate_is_original_receipt_without_second_response");
    Ok(())
}

async fn replay_every_revision(
    runtime: &mut Runtime,
    scope: &Scope,
    initial: &v1::ConversationSnapshot,
    latest: &v1::ConversationSnapshot,
) -> Result {
    let mut cursor = initial.resume_cursor.clone();
    for revision in initial.conversation_revision + 1..=latest.conversation_revision {
        let mut stream = runtime
            .api
            .watch_conversation(v1::WatchConversationRequest {
                workspace_id: scope.workspace.clone(),
                project_id: scope.project.clone(),
                conversation_id: scope.conversation.clone(),
                after_cursor: Some(cursor.clone()),
            })
            .await?
            .into_inner();
        let response = tokio::time::timeout(Duration::from_secs(3), stream.message())
            .await??
            .ok_or("replay lost committed update")?;
        let Some(watch_conversation_response::Frame::Update(update)) = response.frame else {
            return Err("replay returned a non-update".into());
        };
        assert_eq!(update.conversation_revision, revision);
        assert_ne!(update.cursor, cursor);
        cursor = update.cursor;
    }
    assert_eq!(cursor, latest.resume_cursor);
    Ok(())
}
