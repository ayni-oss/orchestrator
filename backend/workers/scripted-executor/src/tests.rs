use super::*;
use ayni_orchestrator_events::EventIdentity;
use ayni_orchestrator_events::v1::{
    ConversationReportDecision, ConversationReportDecisionKind, ConversationTurnBinding,
    ConversationTurnReport, ConversationTurnReportKind, ConversationTurnRequested,
};
use ayni_orchestrator_events::workflow::{
    APPLICATION_SOURCE, WorkflowEventMetadata, decode_report, encode_command, encode_decision,
    encode_report, sign_report, signing_key_digest,
};
use ed25519_dalek::SigningKey;
use tempfile::TempDir;

const WINNER: &str = "018f0f7b-6a4d-7a11-8d77-010203040506";
const LOSER: &str = "018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6";
const WINNER_SEED: [u8; 32] = [7; 32];
const LOSER_SEED: [u8; 32] = [9; 32];
const COMMAND_ID: &str = "018f0f7b-6a4d-7a11-8d77-111213141516";
const CLAIM_ID: &str = "018f0f7b-6a4d-7a11-8d77-818283848586";
const TERMINAL_ID: &str = "018f0f7b-6a4d-7a11-8d77-919293949596";

fn event_metadata() -> WorkflowEventMetadata {
    WorkflowEventMetadata {
        produced_at: "2026-10-20T08:00:00+00:00".into(),
        traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into(),
    }
}

struct CommandFixture {
    identity: EventIdentity,
    binding: ConversationTurnBinding,
    bytes: Vec<u8>,
}

impl CommandFixture {
    fn command(&self) -> Command<'_> {
        Command {
            source: &self.identity.source,
            id: &self.identity.id,
            exact_bytes: &self.bytes,
            turn_id: &self.binding.turn_id,
            assignment_id: &self.binding.assignment_id,
            executor_generation: self.binding.executor_generation,
        }
    }
}

struct ReportFixture {
    identity: EventIdentity,
    bytes: Vec<u8>,
}

impl ReportFixture {
    fn report(&self, sequence: u8) -> Report<'_> {
        Report {
            source: &self.identity.source,
            id: &self.identity.id,
            exact_bytes: &self.bytes,
            sequence,
        }
    }
}

fn open(
    directory: &TempDir,
    limits: LedgerLimits,
    instance: &str,
    seed: &[u8; 32],
) -> Result<Ledger, LedgerError> {
    Ledger::open(
        directory.path().join("ledger.sqlite"),
        limits,
        instance,
        seed,
    )
}

fn command_fixture(generation: u64, event_id: &str) -> Result<CommandFixture, LedgerError> {
    command_fixture_with_input(generation, event_id, "fixture input")
}

fn command_fixture_with_input(
    generation: u64,
    event_id: &str,
    input_content: &str,
) -> Result<CommandFixture, LedgerError> {
    let identity = EventIdentity {
        source: APPLICATION_SOURCE.into(),
        id: event_id.into(),
    };
    let binding = ConversationTurnBinding {
        workspace_id: "workspace:018f0f7b-6a4d-7a11-8d77-212223242526".into(),
        project_id: "project:018f0f7b-6a4d-7a11-8d77-313233343536".into(),
        conversation_id: "conversation:018f0f7b-6a4d-7a11-8d77-414243444546".into(),
        turn_id: "turn:018f0f7b-6a4d-7a11-8d77-515253545556".into(),
        assignment_id: "018f0f7b-6a4d-7a11-8d77-616263646566".into(),
        executor_generation: generation,
        execution_binding_revision: "scripted-v1".into(),
        context_revision: 1,
        command_event_id: identity.id.clone(),
        command_digest: String::new(),
    };
    let payload = ConversationTurnRequested {
        binding: Some(binding.clone()),
        input_message_id: "message:018f0f7b-6a4d-7a11-8d77-717273747576".into(),
        input_content: input_content.into(),
        context: vec![],
    };
    let bytes =
        encode_command(&identity, &event_metadata(), &payload).map_err(|_| LedgerError::Invalid)?;
    Ok(CommandFixture {
        identity,
        binding,
        bytes,
    })
}

fn claim_fixture(
    command: &CommandFixture,
    executor: &str,
    seed: &[u8; 32],
    report_id: &str,
) -> Result<ReportFixture, LedgerError> {
    let identity = EventIdentity {
        source: format!("ayni://orchestrator/supervisors/{executor}"),
        id: report_id.into(),
    };
    let public_key = SigningKey::from_bytes(seed).verifying_key().to_bytes();
    let mut binding = command.binding.clone();
    binding.command_digest = exact_digest(&command.bytes);
    let mut payload = ConversationTurnReport {
        binding: Some(binding),
        kind: ConversationTurnReportKind::Started.into(),
        report_sequence: 1,
        executor_instance_id: executor.into(),
        signing_public_key: public_key.to_vec(),
        signing_key_digest: signing_key_digest(&public_key),
        ..Default::default()
    };
    sign_report(&identity, &mut payload, seed).map_err(|_| LedgerError::Invalid)?;
    let bytes =
        encode_report(&identity, &event_metadata(), &payload).map_err(|_| LedgerError::Invalid)?;
    Ok(ReportFixture { identity, bytes })
}

fn terminal_fixture(
    command: &CommandFixture,
    executor: &str,
    seed: &[u8; 32],
    report_id: &str,
) -> Result<ReportFixture, LedgerError> {
    terminal_fixture_with_binding(
        command,
        executor,
        seed,
        report_id,
        &command.binding.execution_binding_revision,
        command.binding.context_revision,
    )
}

fn terminal_fixture_with_binding(
    command: &CommandFixture,
    executor: &str,
    seed: &[u8; 32],
    report_id: &str,
    execution_binding_revision: &str,
    context_revision: u64,
) -> Result<ReportFixture, LedgerError> {
    let identity = EventIdentity {
        source: format!("ayni://orchestrator/supervisors/{executor}"),
        id: report_id.into(),
    };
    let public_key = SigningKey::from_bytes(seed).verifying_key().to_bytes();
    let mut binding = command.binding.clone();
    binding.command_digest = exact_digest(&command.bytes);
    binding.execution_binding_revision = execution_binding_revision.into();
    binding.context_revision = context_revision;
    let mut payload = ConversationTurnReport {
        binding: Some(binding),
        kind: ConversationTurnReportKind::Completed.into(),
        report_sequence: 2,
        executor_instance_id: executor.into(),
        signing_key_digest: signing_key_digest(&public_key),
        response_content: "scripted response".into(),
        ..Default::default()
    };
    sign_report(&identity, &mut payload, seed).map_err(|_| LedgerError::Invalid)?;
    let bytes =
        encode_report(&identity, &event_metadata(), &payload).map_err(|_| LedgerError::Invalid)?;
    Ok(ReportFixture { identity, bytes })
}

fn decision_bytes(
    command: &CommandFixture,
    report: &ReportFixture,
    target: &str,
    accepted_claimant: &str,
    accepted: bool,
    report_digest: Option<&str>,
    generation: Option<u64>,
) -> Result<Vec<u8>, LedgerError> {
    let (_, _, report_payload) = decode_report(&report.bytes).map_err(|_| LedgerError::Invalid)?;
    let turn_state = match ConversationTurnReportKind::try_from(report_payload.kind)
        .map_err(|_| LedgerError::Invalid)?
    {
        ConversationTurnReportKind::Started => "processing",
        ConversationTurnReportKind::Completed => "completed",
        ConversationTurnReportKind::Failed => "failed",
        ConversationTurnReportKind::Unspecified => return Err(LedgerError::Invalid),
    };
    let payload = ConversationReportDecision {
        report_source: report.identity.source.clone(),
        report_id: report.identity.id.clone(),
        report_digest: report_digest.map_or_else(|| exact_digest(&report.bytes), str::to_owned),
        target_executor_instance_id: target.into(),
        accepted_executor_instance_id: accepted_claimant.into(),
        assignment_id: command.binding.assignment_id.clone(),
        executor_generation: generation.unwrap_or(command.binding.executor_generation),
        decision: if accepted {
            ConversationReportDecisionKind::Accepted.into()
        } else {
            ConversationReportDecisionKind::Rejected.into()
        },
        safe_reason: if accepted {
            String::new()
        } else {
            "claim_lost".into()
        },
        turn_state: turn_state.into(),
        workspace_id: command.binding.workspace_id.clone(),
        project_id: command.binding.project_id.clone(),
        conversation_id: command.binding.conversation_id.clone(),
        turn_id: command.binding.turn_id.clone(),
    };
    let identity = EventIdentity {
        source: APPLICATION_SOURCE.into(),
        id: report.identity.id.clone(),
    };
    encode_decision(&identity, &event_metadata(), &payload).map_err(|_| LedgerError::Invalid)
}

#[test]
fn crash_reopen_preserves_claim_and_blocks_execution_until_decision() -> Result<(), LedgerError> {
    let directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let command = command_fixture(1, COMMAND_ID)?;
    let claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    {
        let mut ledger = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
        assert_eq!(
            ledger.retain_command_and_claim(&command.command(), &claim.report(1))?,
            StoreOutcome::Stored
        );
        assert!(!ledger.can_execute(&command.identity.source, &command.identity.id)?);
    }
    let ledger = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
    assert_eq!(
        ledger.exact_report_bytes(&claim.identity.source, &claim.identity.id)?,
        claim.bytes
    );
    Ok(())
}

#[test]
fn decisions_are_bound_and_duplicates_survive_cleanup() -> Result<(), LedgerError> {
    let directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let command = command_fixture(1, COMMAND_ID)?;
    let claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    let mut ledger = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
    ledger.retain_command_and_claim(&command.command(), &claim.report(1))?;

    let wrong_digest = format!("sha256:{}", "0".repeat(64));
    let wrong = decision_bytes(
        &command,
        &claim,
        WINNER,
        WINNER,
        true,
        Some(&wrong_digest),
        None,
    )?;
    assert!(matches!(
        ledger.record_decision(&Decision {
            exact_bytes: &wrong
        }),
        Err(LedgerError::Invalid)
    ));
    let stale = decision_bytes(&command, &claim, WINNER, WINNER, true, None, Some(2))?;
    assert!(matches!(
        ledger.record_decision(&Decision {
            exact_bytes: &stale
        }),
        Err(LedgerError::Invalid)
    ));

    let accepted = decision_bytes(&command, &claim, WINNER, WINNER, true, None, None)?;
    assert_eq!(
        ledger.record_decision(&Decision {
            exact_bytes: &accepted,
        })?,
        StoreOutcome::Stored
    );
    assert_eq!(
        ledger.retain_command_and_claim(&command.command(), &claim.report(1))?,
        StoreOutcome::Duplicate
    );
    ledger.acknowledge_decision(&claim.identity.source, &claim.identity.id)?;
    assert_eq!(
        ledger.record_decision(&Decision {
            exact_bytes: &accepted,
        })?,
        StoreOutcome::Duplicate
    );
    assert!(ledger.can_execute(&command.identity.source, &command.identity.id)?);
    assert_eq!(
        ledger.retain_redelivered_command(&command.command())?,
        StoreOutcome::Duplicate
    );
    let changed_command = command_fixture_with_input(1, COMMAND_ID, "changed input")?;
    assert!(matches!(
        ledger.retain_redelivered_command(&changed_command.command()),
        Err(LedgerError::EventConflict)
    ));
    Ok(())
}

#[test]
fn terminal_report_and_decision_replay_after_restart() -> Result<(), LedgerError> {
    let directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let command = command_fixture(1, COMMAND_ID)?;
    let claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    let terminal = terminal_fixture(&command, WINNER, &WINNER_SEED, TERMINAL_ID)?;
    let accepted = decision_bytes(&command, &claim, WINNER, WINNER, true, None, None)?;
    {
        let mut ledger = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
        ledger.retain_command_and_claim(&command.command(), &claim.report(1))?;
        ledger.record_decision(&Decision {
            exact_bytes: &accepted,
        })?;
        ledger.retain_terminal_report(
            &command.identity.source,
            &command.identity.id,
            &terminal.report(2),
        )?;
        assert!(!ledger.can_execute(&command.identity.source, &command.identity.id)?);
    }
    let mut reopened = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
    assert_eq!(
        reopened.exact_report_bytes(&terminal.identity.source, &terminal.identity.id)?,
        terminal.bytes
    );
    let terminal_decision = decision_bytes(&command, &terminal, WINNER, WINNER, true, None, None)?;
    reopened.record_decision(&Decision {
        exact_bytes: &terminal_decision,
    })?;
    assert_eq!(
        reopened.retain_terminal_report(
            &command.identity.source,
            &command.identity.id,
            &terminal.report(2),
        )?,
        StoreOutcome::Duplicate
    );
    reopened.acknowledge_decision(&terminal.identity.source, &terminal.identity.id)?;
    assert_eq!(
        reopened.record_decision(&Decision {
            exact_bytes: &terminal_decision,
        })?,
        StoreOutcome::Duplicate
    );
    Ok(())
}

#[test]
fn split_brain_allows_only_the_accepted_ledger_to_execute() -> Result<(), LedgerError> {
    let winner_directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let loser_directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let command = command_fixture(1, COMMAND_ID)?;
    let winner_claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    let loser_claim = claim_fixture(
        &command,
        LOSER,
        &LOSER_SEED,
        "018f0f7b-6a4d-7a11-8d77-c1c2c3c4c5c6",
    )?;
    let mut winner = open(
        &winner_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    let mut loser = open(
        &loser_directory,
        LedgerLimits::m1_default(),
        LOSER,
        &LOSER_SEED,
    )?;
    winner.retain_command_and_claim(&command.command(), &winner_claim.report(1))?;
    loser.retain_command_and_claim(&command.command(), &loser_claim.report(1))?;
    let winner_decision =
        decision_bytes(&command, &winner_claim, WINNER, WINNER, true, None, None)?;
    let loser_decision = decision_bytes(&command, &loser_claim, LOSER, WINNER, false, None, None)?;
    winner.record_decision(&Decision {
        exact_bytes: &winner_decision,
    })?;
    loser.record_decision(&Decision {
        exact_bytes: &loser_decision,
    })?;
    assert!(winner.can_execute(&command.identity.source, &command.identity.id)?);
    assert!(!loser.can_execute(&command.identity.source, &command.identity.id)?);
    let terminal = terminal_fixture(&command, LOSER, &LOSER_SEED, TERMINAL_ID)?;
    assert!(matches!(
        loser.retain_terminal_report(
            &command.identity.source,
            &command.identity.id,
            &terminal.report(2)
        ),
        Err(LedgerError::ClaimNotAccepted)
    ));
    Ok(())
}

#[test]
fn accepted_owner_cannot_rotate_but_an_unaccepted_generation_can() -> Result<(), LedgerError> {
    let generation_one = command_fixture(1, COMMAND_ID)?;
    let claim_one = claim_fixture(&generation_one, WINNER, &WINNER_SEED, CLAIM_ID)?;
    let generation_two = command_fixture(2, "018f0f7b-6a4d-7a11-8d77-d1d2d3d4d5d6")?;
    let claim_two = claim_fixture(
        &generation_two,
        WINNER,
        &WINNER_SEED,
        "018f0f7b-6a4d-7a11-8d77-e1e2e3e4e5e6",
    )?;
    let decision_one = decision_bytes(
        &generation_one,
        &claim_one,
        WINNER,
        WINNER,
        true,
        None,
        None,
    )?;

    let accepted_directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let mut accepted = open(
        &accepted_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    accepted.retain_command_and_claim(&generation_one.command(), &claim_one.report(1))?;
    accepted.record_decision(&Decision {
        exact_bytes: &decision_one,
    })?;
    assert!(matches!(
        accepted.retain_command_and_claim(&generation_two.command(), &claim_two.report(1)),
        Err(LedgerError::AcceptedOwnerCannotRotate)
    ));
    assert!(accepted.can_execute(&generation_one.identity.source, &generation_one.identity.id)?);

    let pending_directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let mut pending = open(
        &pending_directory,
        LedgerLimits::m1_default(),
        WINNER,
        &WINNER_SEED,
    )?;
    pending.retain_command_and_claim(&generation_one.command(), &claim_one.report(1))?;
    pending.retain_command_and_claim(&generation_two.command(), &claim_two.report(1))?;
    assert!(matches!(
        pending.record_decision(&Decision {
            exact_bytes: &decision_one,
        }),
        Err(LedgerError::StaleGeneration)
    ));
    Ok(())
}

#[test]
fn terminal_report_requires_the_full_frozen_binding() -> Result<(), LedgerError> {
    let directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let mut ledger = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
    let command = command_fixture(1, COMMAND_ID)?;
    let claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    ledger.retain_command_and_claim(&command.command(), &claim.report(1))?;
    let accepted = decision_bytes(&command, &claim, WINNER, WINNER, true, None, None)?;
    ledger.record_decision(&Decision {
        exact_bytes: &accepted,
    })?;

    let wrong_context = terminal_fixture_with_binding(
        &command,
        WINNER,
        &WINNER_SEED,
        TERMINAL_ID,
        &command.binding.execution_binding_revision,
        command.binding.context_revision + 1,
    )?;
    assert!(matches!(
        ledger.retain_terminal_report(
            &command.identity.source,
            &command.identity.id,
            &wrong_context.report(2),
        ),
        Err(LedgerError::Invalid)
    ));
    Ok(())
}

#[test]
fn exact_byte_conflicts_and_cleanup_are_explicit() -> Result<(), LedgerError> {
    let directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let mut ledger = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
    let command = command_fixture(1, COMMAND_ID)?;
    let claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    ledger.retain_command_and_claim(&command.command(), &claim.report(1))?;

    let changed_command = command_fixture_with_input(1, COMMAND_ID, "changed input")?;
    let changed_command_claim = claim_fixture(&changed_command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    assert!(matches!(
            ledger.retain_command_and_claim(
                &changed_command.command(),
                &changed_command_claim.report(1),
            ),
            Err(LedgerError::EventConflict)
        ));
    let changed_claim = claim_fixture(&command, WINNER, &[8; 32], CLAIM_ID)?;
    assert!(matches!(
        ledger.retain_command_and_claim(&command.command(), &changed_claim.report(1)),
        Err(LedgerError::Invalid)
    ));

    let accepted = decision_bytes(&command, &claim, WINNER, WINNER, true, None, None)?;
    ledger.record_decision(&Decision {
        exact_bytes: &accepted,
    })?;
    let (report_bytes, retained_decision_bytes): (Option<Vec<u8>>, Option<Vec<u8>>) =
        ledger.connection.query_row(
            "SELECT event_bytes, decision_bytes FROM reports WHERE source = ?1 AND event_id = ?2",
            params![claim.identity.source, claim.identity.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
    assert!(report_bytes.is_none());
    assert_eq!(
        retained_decision_bytes.as_deref(),
        Some(accepted.as_slice())
    );

    let changed_decision = decision_bytes(&command, &claim, WINNER, WINNER, false, None, None)?;
    assert!(matches!(
        ledger.record_decision(&Decision {
            exact_bytes: &changed_decision,
        }),
        Err(LedgerError::EventConflict)
    ));
    ledger.acknowledge_decision(&claim.identity.source, &claim.identity.id)?;
    let decision_bytes: Option<Vec<u8>> = ledger.connection.query_row(
        "SELECT decision_bytes FROM reports WHERE source = ?1 AND event_id = ?2",
        params![claim.identity.source, claim.identity.id],
        |row| row.get(0),
    )?;
    assert!(decision_bytes.is_none());
    Ok(())
}

#[test]
fn stale_generation_and_full_ledger_reject_atomically() -> Result<(), LedgerError> {
    let directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let mut ledger = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
    let generation_two = command_fixture(2, "018f0f7b-6a4d-7a11-8d77-d1d2d3d4d5d6")?;
    let claim_two = claim_fixture(
        &generation_two,
        WINNER,
        &WINNER_SEED,
        "018f0f7b-6a4d-7a11-8d77-e1e2e3e4e5e6",
    )?;
    ledger.retain_command_and_claim(&generation_two.command(), &claim_two.report(1))?;
    let stale = command_fixture(1, "018f0f7b-6a4d-7a11-8d77-f1f2f3f4f5f6")?;
    let stale_claim = claim_fixture(
        &stale,
        WINNER,
        &WINNER_SEED,
        "018f0f7b-6a4d-7a11-8d77-0123456789ab",
    )?;
    assert!(matches!(
        ledger.retain_command_and_claim(&stale.command(), &stale_claim.report(1)),
        Err(LedgerError::StaleGeneration)
    ));

    let full_directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let command = command_fixture(1, COMMAND_ID)?;
    let claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    let mut full = open(
        &full_directory,
        LedgerLimits {
            max_records: 2,
            max_exact_bytes: u64::try_from(command.bytes.len() + claim.bytes.len())
                .map_err(|_| LedgerError::Invalid)?,
        },
        WINNER,
        &WINNER_SEED,
    )?;
    assert!(matches!(
        full.retain_command_and_claim(&command.command(), &claim.report(1)),
        Err(LedgerError::Capacity)
    ));
    assert!(!full.can_execute(&command.identity.source, &command.identity.id)?);
    Ok(())
}

#[test]
fn retirement_and_metadata_cleanup_wait_for_every_decision_ack() -> Result<(), LedgerError> {
    let directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let mut ledger = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
    let command = command_fixture(1, COMMAND_ID)?;
    let claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    ledger.retain_command_and_claim(&command.command(), &claim.report(1))?;
    let claim_decision = decision_bytes(&command, &claim, WINNER, WINNER, true, None, None)?;
    ledger.record_decision(&Decision {
        exact_bytes: &claim_decision,
    })?;
    assert!(!ledger.can_retire()?);
    ledger.acknowledge_decision_at(&claim.identity.source, &claim.identity.id, 1_000)?;
    ledger.acknowledge_decision_at(&claim.identity.source, &claim.identity.id, 2_000)?;
    assert_eq!(
        ledger.prune_resolved_metadata(1_000 + RESOLVED_METADATA_RETENTION_SECONDS)?,
        0
    );

    let terminal = terminal_fixture(&command, WINNER, &WINNER_SEED, TERMINAL_ID)?;
    ledger.retain_terminal_report(
        &command.identity.source,
        &command.identity.id,
        &terminal.report(2),
    )?;
    let terminal_decision = decision_bytes(&command, &terminal, WINNER, WINNER, true, None, None)?;
    ledger.record_decision(&Decision {
        exact_bytes: &terminal_decision,
    })?;
    assert!(!ledger.can_retire()?);
    ledger.acknowledge_decision_at(&terminal.identity.source, &terminal.identity.id, 1_000)?;
    assert!(ledger.can_retire()?);

    assert_eq!(
        ledger.prune_resolved_metadata(1_000 + RESOLVED_METADATA_RETENTION_SECONDS - 1)?,
        0
    );
    assert_eq!(
        ledger.prune_resolved_metadata(1_000 + RESOLVED_METADATA_RETENTION_SECONDS)?,
        3
    );
    let retained: i64 = ledger.connection.query_row(
        "SELECT (SELECT count(*) FROM commands) + (SELECT count(*) FROM reports)",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(retained, 0);
    Ok(())
}

#[test]
fn sqlite_full_rolls_back_command_and_claim_atomically() -> Result<(), LedgerError> {
    let directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
    let mut ledger = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
    let page_count: i64 = ledger
        .connection
        .pragma_query_value(None, "page_count", |row| row.get(0))?;
    ledger
        .connection
        .pragma_update(None, "max_page_count", page_count)?;

    let input = "x".repeat(16 * 1024);
    let command = command_fixture_with_input(1, COMMAND_ID, &input)?;
    let claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
    assert!(matches!(
        ledger.retain_command_and_claim(&command.command(), &claim.report(1)),
        Err(LedgerError::Sql(_))
    ));
    let retained: i64 = ledger.connection.query_row(
        "SELECT (SELECT count(*) FROM commands) + (SELECT count(*) FROM reports)",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(retained, 0);
    Ok(())
}

#[path = "transport_tests.rs"]
mod transport_tests;
