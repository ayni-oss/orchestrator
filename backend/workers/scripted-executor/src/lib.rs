//! Retained local ledger prototype for the deterministic M1 executor.
//!
//! The ledger owns exact command and report bytes until the corresponding
//! application decision is durable. It is deliberately synchronous; a future
//! worker process must call it from a blocking boundary.

use ayni_orchestrator_events::EventIdentity;
use ayni_orchestrator_events::v1::{
    ConversationReportDecision, ConversationReportDecisionKind, ConversationTurnBinding,
    ConversationTurnReport, ConversationTurnReportKind,
};
use ayni_orchestrator_events::workflow::{
    decode_command, decode_decision, decode_report, verify_report_signature,
};
use ed25519_dalek::SigningKey;
use rusqlite::{Connection, OptionalExtension as _, Transaction, TransactionBehavior, params};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

mod decision;
mod schema;
mod storage;

use schema::SCHEMA;
use storage::{
    exact_digest, existing_command_outcome, existing_report_outcome, insert_report,
    redelivered_command_outcome, reserve,
};

const CLAIM_REPORT_RESERVATION_BYTES: usize = 24 * 1024;
const TERMINAL_REPORT_RESERVATION_BYTES: usize = 40 * 1024;
const RESOLVED_METADATA_RETENTION_SECONDS: u64 = 8 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug)]
pub struct LedgerLimits {
    pub max_records: u64,
    pub max_exact_bytes: u64,
}

impl LedgerLimits {
    #[must_use]
    pub const fn m1_default() -> Self {
        Self {
            max_records: 50_000,
            max_exact_bytes: 192 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Command<'a> {
    pub source: &'a str,
    pub id: &'a str,
    pub exact_bytes: &'a [u8],
    pub turn_id: &'a str,
    pub assignment_id: &'a str,
    pub executor_generation: u64,
}

#[derive(Clone, Debug)]
pub struct Report<'a> {
    pub source: &'a str,
    pub id: &'a str,
    pub exact_bytes: &'a [u8],
    pub sequence: u8,
}

#[derive(Clone, Debug)]
pub struct Decision<'a> {
    pub exact_bytes: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreOutcome {
    Stored,
    Duplicate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedEvent {
    pub source: String,
    pub id: String,
    pub exact_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandDelivery {
    New,
    PendingClaim(RetainedEvent),
    Resolved,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingDecisionAck {
    pub report_source: String,
    pub report_id: String,
    pub consumer_sequence: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("SQLite ledger error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("ledger identity differs from retained identity")]
    IdentityConflict,
    #[error("event identity was reused with different exact bytes")]
    EventConflict,
    #[error("executor generation is stale")]
    StaleGeneration,
    #[error("an accepted executor owner cannot be replaced in M1")]
    AcceptedOwnerCannotRotate,
    #[error("executor ledger capacity is exhausted")]
    Capacity,
    #[error("the report or command does not exist")]
    NotFound,
    #[error("the decision consumer has not durably acknowledged this delivery")]
    DecisionAckPending,
    #[error("the application has not accepted this executor claim")]
    ClaimNotAccepted,
    #[error("invalid ledger input")]
    Invalid,
}

pub struct Ledger {
    connection: Connection,
    limits: LedgerLimits,
}

struct ValidatedCommand {
    workspace_id: String,
    project_id: String,
    conversation_id: String,
    turn_id: String,
    assignment_id: String,
    generation: u64,
    execution_binding_revision: String,
    context_revision: u64,
}

struct RetainedDecisionReport {
    prior_result: Option<String>,
    prior_digest: Option<String>,
    prior_bytes: Option<Vec<u8>>,
    report_bytes: Option<Vec<u8>>,
    report_reserved_bytes: i64,
    report_digest: String,
    sequence: i64,
    command_source: String,
    command_event_id: String,
    command_bytes: Option<Vec<u8>>,
    terminal_reserved_bytes: i64,
    terminal_reserved_records: i64,
}

struct RetainedCommandBinding {
    command_digest: String,
    workspace_id: String,
    project_id: String,
    conversation_id: String,
    turn_id: String,
    assignment_id: String,
    generation: i64,
    execution_binding_revision: String,
    context_revision: i64,
    signing_public_key: Vec<u8>,
    state: String,
}

struct RetainedDecisionCommand {
    workspace_id: String,
    project_id: String,
    conversation_id: String,
    turn_id: String,
    assignment_id: String,
    generation: i64,
    superseded: bool,
}

impl Ledger {
    /// Open or create a ledger and verify that its retained executor identity is
    /// unchanged. The seed is fixture input here; production provisioning must
    /// generate and inject it without logging.
    ///
    /// # Errors
    /// Returns schema, storage, validation, or identity conflicts.
    pub fn open(
        path: impl AsRef<Path>,
        limits: LedgerLimits,
        instance_id: &str,
        signing_seed: &[u8; 32],
    ) -> Result<Self, LedgerError> {
        if instance_id.is_empty() || limits.max_records == 0 || limits.max_exact_bytes == 0 {
            return Err(LedgerError::Invalid);
        }
        let connection = Connection::open(path)?;
        connection.execute_batch(SCHEMA)?;
        let retained: Option<(String, Vec<u8>)> = connection
            .query_row(
                "SELECT instance_id, signing_seed FROM executor_identity WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((retained_id, retained_seed)) = retained {
            if retained_id != instance_id || retained_seed != signing_seed {
                return Err(LedgerError::IdentityConflict);
            }
        } else {
            connection.execute(
                "INSERT INTO executor_identity (singleton, instance_id, signing_seed) VALUES (1, ?1, ?2)",
                params![instance_id, signing_seed.as_slice()],
            )?;
        }
        Ok(Self { connection, limits })
    }

    /// Atomically retain a command and its sequence-1 ownership claim before a
    /// command-consumer ACK can be sent.
    ///
    /// # Errors
    /// Returns conflicts, stale generations, capacity exhaustion, or SQL errors.
    pub fn retain_command_and_claim(
        &mut self,
        command: &Command<'_>,
        claim: &Report<'_>,
    ) -> Result<StoreOutcome, LedgerError> {
        validate_command(command)?;
        validate_report(claim, 1)?;
        let instance_id = retained_instance_id(&self.connection)?;
        let signing_public_key = retained_signing_public_key(&self.connection)?;
        let validated = validate_command_claim(command, claim, &instance_id, &signing_public_key)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(outcome) = existing_command_outcome(&transaction, command, claim)? {
            transaction.commit()?;
            return Ok(outcome);
        }
        let latest: Option<(i64, String)> = transaction
            .query_row(
                "SELECT generation, state FROM commands \
                 WHERE turn_id = ?1 AND assignment_id = ?2 \
                 ORDER BY generation DESC LIMIT 1",
                params![validated.turn_id, validated.assignment_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let generation = i64::try_from(validated.generation).map_err(|_| LedgerError::Invalid)?;
        if latest
            .as_ref()
            .is_some_and(|(latest, _)| generation < *latest)
        {
            return Err(LedgerError::StaleGeneration);
        }
        if latest
            .as_ref()
            .is_some_and(|(latest, _)| generation == *latest)
        {
            return Err(LedgerError::EventConflict);
        }
        if latest.as_ref().is_some_and(|(_, state)| {
            matches!(
                state.as_str(),
                "accepted" | "terminal_pending" | "completed"
            )
        }) {
            return Err(LedgerError::AcceptedOwnerCannotRotate);
        }
        transaction.execute(
            "UPDATE commands SET state = 'rejected', terminal_reserved_bytes = 0, \
             terminal_reserved_records = 0 \
             WHERE turn_id = ?1 AND assignment_id = ?2 AND generation < ?3",
            params![validated.turn_id, validated.assignment_id, generation],
        )?;
        reserve(
            &transaction,
            self.limits,
            3,
            0,
            command.exact_bytes.len()
                + CLAIM_REPORT_RESERVATION_BYTES
                + TERMINAL_REPORT_RESERVATION_BYTES,
            0,
        )?;
        transaction.execute(
            "INSERT INTO commands \
             (source, event_id, digest, event_bytes, workspace_id, project_id, conversation_id, \
              turn_id, assignment_id, generation, execution_binding_revision, context_revision, \
              terminal_reserved_bytes, terminal_reserved_records, state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 1, 'claim_pending')",
            params![
                command.source,
                command.id,
                exact_digest(command.exact_bytes),
                command.exact_bytes,
                validated.workspace_id,
                validated.project_id,
                validated.conversation_id,
                validated.turn_id,
                validated.assignment_id,
                generation,
                validated.execution_binding_revision,
                i64::try_from(validated.context_revision).map_err(|_| LedgerError::Invalid)?,
                i64::try_from(TERMINAL_REPORT_RESERVATION_BYTES)
                    .map_err(|_| LedgerError::Invalid)?,
            ],
        )?;
        insert_report(&transaction, command, claim)?;
        transaction.commit()?;
        Ok(StoreOutcome::Stored)
    }

    /// Validate a redelivered command after its claim bytes may already have
    /// been handed off and cleaned. No new claim is created and execution state
    /// is unchanged.
    ///
    /// # Errors
    /// Returns missing commands, changed-byte conflicts, validation, or SQL errors.
    pub fn retain_redelivered_command(
        &mut self,
        command: &Command<'_>,
    ) -> Result<StoreOutcome, LedgerError> {
        validate_redelivered_command(command)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let outcome = redelivered_command_outcome(&transaction, command)?;
        transaction.commit()?;
        Ok(outcome)
    }

    /// Retain one sequence-2 terminal report. Execution is forbidden until the
    /// sequence-1 claim decision is durably accepted.
    ///
    /// # Errors
    /// Returns missing/unaccepted claims, conflicts, capacity, or SQL errors.
    pub fn retain_terminal_report(
        &mut self,
        command_source: &str,
        command_event_id: &str,
        report: &Report<'_>,
    ) -> Result<StoreOutcome, LedgerError> {
        validate_report(report, 2)?;
        let instance_id = retained_instance_id(&self.connection)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(outcome) =
            existing_report_outcome(&transaction, command_source, command_event_id, report)?
        {
            transaction.commit()?;
            return Ok(outcome);
        }
        validate_terminal_report(
            &transaction,
            command_source,
            command_event_id,
            report,
            &instance_id,
        )?;
        reserve(
            &transaction,
            self.limits,
            1,
            1,
            TERMINAL_REPORT_RESERVATION_BYTES,
            TERMINAL_REPORT_RESERVATION_BYTES,
        )?;
        let command = Command {
            source: command_source,
            id: command_event_id,
            exact_bytes: &[],
            turn_id: "retained",
            assignment_id: "retained",
            executor_generation: 1,
        };
        insert_report(&transaction, &command, report)?;
        transaction.execute(
            "UPDATE commands SET state = 'terminal_pending', terminal_reserved_bytes = 0, \
             terminal_reserved_records = 0 \
             WHERE source = ?1 AND event_id = ?2",
            params![command_source, command_event_id],
        )?;
        transaction.commit()?;
        Ok(StoreOutcome::Stored)
    }

    /// Remove resolved identity metadata only after the fixed eight-day replay
    /// horizon. Unresolved commands, reports, and exact bytes are never pruned.
    ///
    /// # Errors
    /// Returns SQL or invalid-clock failures.
    pub fn prune_resolved_metadata(&mut self, now_seconds: u64) -> Result<u64, LedgerError> {
        let cutoff = now_seconds.saturating_sub(RESOLVED_METADATA_RETENTION_SECONDS);
        let cutoff = i64::try_from(cutoff).map_err(|_| LedgerError::Invalid)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reports = transaction.execute(
            "DELETE FROM reports WHERE decision_acked = 1 \
             AND resolved_at_seconds <= ?1 \
             AND EXISTS (SELECT 1 FROM commands \
               WHERE commands.source = reports.command_source \
               AND commands.event_id = reports.command_event_id \
               AND commands.state IN ('rejected', 'completed'))",
            params![cutoff],
        )?;
        let commands = transaction.execute(
            "DELETE FROM commands WHERE state IN ('rejected', 'completed') \
             AND NOT EXISTS (SELECT 1 FROM reports \
               WHERE reports.command_source = commands.source \
               AND reports.command_event_id = commands.event_id)",
            [],
        )?;
        transaction.commit()?;
        u64::try_from(reports.saturating_add(commands)).map_err(|_| LedgerError::Invalid)
    }

    /// Retirement is safe only when no execution is unresolved and every
    /// targeted application decision has been durably `ACKed`.
    ///
    /// # Errors
    /// Returns SQL failures.
    pub fn can_retire(&self) -> Result<bool, LedgerError> {
        Ok(self.connection.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM commands \
               WHERE state IN ('claim_pending', 'accepted', 'terminal_pending')) \
             AND NOT EXISTS(SELECT 1 FROM reports \
               WHERE decision IS NULL OR decision_acked = 0)",
            [],
            |row| row.get(0),
        )?)
    }

    /// True only after an accepted sequence-1 decision is durable and while the
    /// exact command needed for deterministic execution is retained.
    ///
    /// # Errors
    /// Returns SQL failures.
    pub fn can_execute(
        &self,
        command_source: &str,
        command_event_id: &str,
    ) -> Result<bool, LedgerError> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands WHERE source = ?1 AND event_id = ?2 \
             AND state = 'accepted' AND event_bytes IS NOT NULL)",
            params![command_source, command_event_id],
            |row| row.get(0),
        )?)
    }

    /// Return retained exact bytes for crash/restart proof.
    ///
    /// # Errors
    /// Returns SQL failures or missing/cleaned records.
    pub fn exact_report_bytes(
        &self,
        report_source: &str,
        report_event_id: &str,
    ) -> Result<Vec<u8>, LedgerError> {
        self.connection
            .query_row(
                "SELECT event_bytes FROM reports WHERE source = ?1 AND event_id = ?2",
                params![report_source, report_event_id],
                |row| row.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
            .flatten()
            .ok_or(LedgerError::NotFound)
    }

    /// Classify an exact command delivery without creating a second claim.
    ///
    /// # Errors
    /// Returns validation, changed-byte conflicts, or `SQLite` failures.
    pub fn classify_command_delivery(
        &mut self,
        command: &Command<'_>,
    ) -> Result<CommandDelivery, LedgerError> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands WHERE source = ?1 AND event_id = ?2)",
            params![command.source, command.id],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(CommandDelivery::New);
        }
        self.retain_redelivered_command(command)?;
        let claim: Option<(String, String, Option<Vec<u8>>)> = self
            .connection
            .query_row(
                "SELECT source, event_id, event_bytes FROM reports \
                 WHERE command_source = ?1 AND command_event_id = ?2 AND sequence = 1",
                params![command.source, command.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        match claim {
            Some((source, id, Some(exact_bytes))) => {
                Ok(CommandDelivery::PendingClaim(RetainedEvent {
                    source,
                    id,
                    exact_bytes,
                }))
            }
            Some((_, _, None)) => Ok(CommandDelivery::Resolved),
            None => Err(LedgerError::EventConflict),
        }
    }

    /// Return all unresolved exact report bytes for reconnect publication.
    ///
    /// # Errors
    /// Returns `SQLite` failures or invalid retained state.
    pub fn pending_reports(&self) -> Result<Vec<RetainedEvent>, LedgerError> {
        let mut statement = self.connection.prepare(
            "SELECT source, event_id, event_bytes FROM reports \
             WHERE decision IS NULL AND event_bytes IS NOT NULL \
             ORDER BY rowid",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(RetainedEvent {
                source: row.get(0)?,
                id: row.get(1)?,
                exact_bytes: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Return commands authorized for deterministic execution after restart.
    ///
    /// # Errors
    /// Returns `SQLite` failures or invalid retained state.
    pub fn executable_commands(&self) -> Result<Vec<RetainedEvent>, LedgerError> {
        let mut statement = self.connection.prepare(
            "SELECT source, event_id, event_bytes FROM commands \
             WHERE state = 'accepted' AND event_bytes IS NOT NULL ORDER BY rowid",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(RetainedEvent {
                source: row.get(0)?,
                id: row.get(1)?,
                exact_bytes: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Return decisions committed locally but not yet reconciled with the
    /// server-confirmed consumer acknowledgement floor.
    ///
    /// # Errors
    /// Returns `SQLite` failures or invalid retained sequences.
    pub fn pending_decision_acknowledgements(
        &self,
    ) -> Result<Vec<PendingDecisionAck>, LedgerError> {
        let mut statement = self.connection.prepare(
            "SELECT source, event_id, decision_consumer_sequence FROM reports \
             WHERE decision IS NOT NULL AND decision_acked = 0 \
               AND decision_consumer_sequence IS NOT NULL ORDER BY decision_consumer_sequence",
        )?;
        let rows = statement.query_map([], |row| {
            let consumer_sequence: i64 = row.get(2)?;
            Ok((row.get(0)?, row.get(1)?, consumer_sequence))
        })?;
        rows.map(|row| {
            let (report_source, report_id, consumer_sequence) = row?;
            Ok(PendingDecisionAck {
                report_source,
                report_id,
                consumer_sequence: u64::try_from(consumer_sequence)
                    .map_err(|_| LedgerError::Invalid)?,
            })
        })
        .collect()
    }
}

fn unix_seconds() -> Result<u64, LedgerError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| LedgerError::Invalid)
}

fn validate_redelivered_command(command: &Command<'_>) -> Result<(), LedgerError> {
    validate_command(command)?;
    let (identity, _, payload) =
        decode_command(command.exact_bytes).map_err(|_| LedgerError::Invalid)?;
    let binding = payload.binding.ok_or(LedgerError::Invalid)?;
    if identity.source != command.source
        || identity.id != command.id
        || binding.turn_id != command.turn_id
        || binding.assignment_id != command.assignment_id
        || binding.executor_generation != command.executor_generation
    {
        return Err(LedgerError::Invalid);
    }
    Ok(())
}

const fn validate_command(command: &Command<'_>) -> Result<(), LedgerError> {
    if command.source.is_empty()
        || command.id.is_empty()
        || command.exact_bytes.is_empty()
        || command.turn_id.is_empty()
        || command.assignment_id.is_empty()
        || command.executor_generation == 0
    {
        return Err(LedgerError::Invalid);
    }
    Ok(())
}

const fn validate_report(report: &Report<'_>, sequence: u8) -> Result<(), LedgerError> {
    if report.source.is_empty()
        || report.id.is_empty()
        || report.exact_bytes.is_empty()
        || report.sequence != sequence
    {
        return Err(LedgerError::Invalid);
    }
    Ok(())
}

fn retained_instance_id(connection: &Connection) -> Result<String, LedgerError> {
    Ok(connection.query_row(
        "SELECT instance_id FROM executor_identity WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?)
}

fn retained_signing_public_key(connection: &Connection) -> Result<[u8; 32], LedgerError> {
    let seed: Vec<u8> = connection.query_row(
        "SELECT signing_seed FROM executor_identity WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    let seed: [u8; 32] = seed.try_into().map_err(|_| LedgerError::Invalid)?;
    Ok(SigningKey::from_bytes(&seed).verifying_key().to_bytes())
}

fn decode_targeted_decision(
    exact_bytes: &[u8],
    instance_id: &str,
) -> Result<(ConversationReportDecision, bool), LedgerError> {
    if exact_bytes.is_empty() {
        return Err(LedgerError::Invalid);
    }
    let (_, _, payload) = decode_decision(exact_bytes).map_err(|_| LedgerError::Invalid)?;
    if payload.target_executor_instance_id != instance_id {
        return Err(LedgerError::Invalid);
    }
    let accepted = match ConversationReportDecisionKind::try_from(payload.decision)
        .map_err(|_| LedgerError::Invalid)?
    {
        ConversationReportDecisionKind::Accepted => true,
        ConversationReportDecisionKind::Rejected => false,
        ConversationReportDecisionKind::Unspecified => return Err(LedgerError::Invalid),
    };
    Ok((payload, accepted))
}

fn retain_decision_consumer_sequence(
    transaction: &Transaction<'_>,
    report_source: &str,
    report_event_id: &str,
    consumer_sequence: Option<u64>,
) -> Result<(), LedgerError> {
    let Some(consumer_sequence) = consumer_sequence else {
        return Ok(());
    };
    let consumer_sequence = i64::try_from(consumer_sequence).map_err(|_| LedgerError::Invalid)?;
    transaction.execute(
        "UPDATE reports SET decision_consumer_sequence = \
         CASE WHEN COALESCE(decision_consumer_sequence, 0) < ?3 THEN ?3 \
              ELSE decision_consumer_sequence END \
         WHERE source = ?1 AND event_id = ?2",
        params![report_source, report_event_id, consumer_sequence],
    )?;
    Ok(())
}

fn retained_decision_report(
    transaction: &Transaction<'_>,
    decision: &ConversationReportDecision,
) -> Result<RetainedDecisionReport, LedgerError> {
    transaction
        .query_row(
            "SELECT reports.decision, reports.decision_digest, reports.decision_bytes, \
             reports.event_bytes, reports.reserved_bytes, reports.digest, reports.sequence, \
             reports.command_source, reports.command_event_id, commands.event_bytes, \
             commands.terminal_reserved_bytes, commands.terminal_reserved_records \
             FROM reports JOIN commands \
               ON commands.source = reports.command_source \
              AND commands.event_id = reports.command_event_id \
             WHERE reports.source = ?1 AND reports.event_id = ?2",
            params![decision.report_source, decision.report_id],
            |row| {
                Ok(RetainedDecisionReport {
                    prior_result: row.get(0)?,
                    prior_digest: row.get(1)?,
                    prior_bytes: row.get(2)?,
                    report_bytes: row.get(3)?,
                    report_reserved_bytes: row.get(4)?,
                    report_digest: row.get(5)?,
                    sequence: row.get(6)?,
                    command_source: row.get(7)?,
                    command_event_id: row.get(8)?,
                    command_bytes: row.get(9)?,
                    terminal_reserved_bytes: row.get(10)?,
                    terminal_reserved_records: row.get(11)?,
                })
            },
        )
        .optional()?
        .ok_or(LedgerError::NotFound)
}

fn prior_decision_is_duplicate(
    retained: &RetainedDecisionReport,
    result: &str,
    decision_digest: &str,
    decision_bytes: &[u8],
) -> Result<bool, LedgerError> {
    let Some(prior_result) = retained.prior_result.as_deref() else {
        return Ok(false);
    };
    if prior_result == result
        && retained.prior_digest.as_deref() == Some(decision_digest)
        && retained
            .prior_bytes
            .as_deref()
            .is_none_or(|bytes| bytes == decision_bytes)
    {
        Ok(true)
    } else {
        Err(LedgerError::EventConflict)
    }
}

const fn command_transition(
    sequence: i64,
    accepted: bool,
) -> Result<(&'static str, bool), LedgerError> {
    match (sequence, accepted) {
        (1, true) => Ok(("accepted", false)),
        (1 | 2, false) => Ok(("rejected", true)),
        (2, true) => Ok(("completed", true)),
        _ => Err(LedgerError::Invalid),
    }
}

fn validate_command_claim(
    command: &Command<'_>,
    claim: &Report<'_>,
    instance_id: &str,
    signing_public_key: &[u8; 32],
) -> Result<ValidatedCommand, LedgerError> {
    let (command_identity, _, command_payload) =
        decode_command(command.exact_bytes).map_err(|_| LedgerError::Invalid)?;
    let command_binding = command_payload.binding.ok_or(LedgerError::Invalid)?;
    let command_identity_matches =
        command_identity.source == command.source && command_identity.id == command.id;
    if !command_identity_matches
        || command_binding.turn_id != command.turn_id
        || command_binding.assignment_id != command.assignment_id
        || command_binding.executor_generation != command.executor_generation
    {
        return Err(LedgerError::Invalid);
    }
    let (claim_identity, _, claim_payload) =
        decode_report(claim.exact_bytes).map_err(|_| LedgerError::Invalid)?;
    let claim_binding = claim_payload.binding.as_ref().ok_or(LedgerError::Invalid)?;
    let mut expected_claim_binding = command_binding.clone();
    expected_claim_binding.command_digest = exact_digest(command.exact_bytes);
    let claim_identity_matches =
        claim_identity.source == claim.source && claim_identity.id == claim.id;
    if !claim_identity_matches
        || claim_payload.report_sequence != 1
        || ConversationTurnReportKind::try_from(claim_payload.kind)
            .map_err(|_| LedgerError::Invalid)?
            != ConversationTurnReportKind::Started
        || claim_payload.executor_instance_id != instance_id
        || claim_payload.signing_public_key.as_slice() != signing_public_key
        || claim_binding != &expected_claim_binding
    {
        return Err(LedgerError::Invalid);
    }
    verify_report_signature(&claim_identity, &claim_payload, None)
        .map_err(|_| LedgerError::Invalid)?;
    Ok(ValidatedCommand {
        workspace_id: command_binding.workspace_id,
        project_id: command_binding.project_id,
        conversation_id: command_binding.conversation_id,
        turn_id: command_binding.turn_id,
        assignment_id: command_binding.assignment_id,
        generation: command_binding.executor_generation,
        execution_binding_revision: command_binding.execution_binding_revision,
        context_revision: command_binding.context_revision,
    })
}

fn validate_terminal_report(
    transaction: &Transaction<'_>,
    command_source: &str,
    command_event_id: &str,
    report: &Report<'_>,
    instance_id: &str,
) -> Result<(), LedgerError> {
    let retained = retained_command_binding(transaction, command_source, command_event_id)?;
    if retained.state != "accepted" {
        return Err(LedgerError::ClaimNotAccepted);
    }
    let (identity, _, payload) =
        decode_report(report.exact_bytes).map_err(|_| LedgerError::Invalid)?;
    let binding = payload.binding.as_ref().ok_or(LedgerError::Invalid)?;
    let kind =
        ConversationTurnReportKind::try_from(payload.kind).map_err(|_| LedgerError::Invalid)?;
    let public_key: [u8; 32] = retained
        .signing_public_key
        .as_slice()
        .try_into()
        .map_err(|_| LedgerError::Invalid)?;
    if !terminal_report_header_matches(report, &identity, &payload, kind, instance_id)
        || !terminal_binding_matches(binding, &retained, command_event_id)?
    {
        return Err(LedgerError::Invalid);
    }
    verify_report_signature(&identity, &payload, Some(&public_key))
        .map_err(|_| LedgerError::Invalid)
}

fn retained_command_binding(
    transaction: &Transaction<'_>,
    command_source: &str,
    command_event_id: &str,
) -> Result<RetainedCommandBinding, LedgerError> {
    transaction
        .query_row(
            "SELECT digest, workspace_id, project_id, conversation_id, turn_id, assignment_id, \
             generation, execution_binding_revision, context_revision, signing_public_key, state \
             FROM commands WHERE source = ?1 AND event_id = ?2",
            params![command_source, command_event_id],
            |row| {
                Ok(RetainedCommandBinding {
                    command_digest: row.get(0)?,
                    workspace_id: row.get(1)?,
                    project_id: row.get(2)?,
                    conversation_id: row.get(3)?,
                    turn_id: row.get(4)?,
                    assignment_id: row.get(5)?,
                    generation: row.get(6)?,
                    execution_binding_revision: row.get(7)?,
                    context_revision: row.get(8)?,
                    signing_public_key: row.get(9)?,
                    state: row.get(10)?,
                })
            },
        )
        .optional()?
        .ok_or(LedgerError::NotFound)
}

fn terminal_report_header_matches(
    report: &Report<'_>,
    identity: &EventIdentity,
    payload: &ConversationTurnReport,
    kind: ConversationTurnReportKind,
    instance_id: &str,
) -> bool {
    identity.source == report.source
        && identity.id == report.id
        && payload.executor_instance_id == instance_id
        && payload.report_sequence == 2
        && matches!(
            kind,
            ConversationTurnReportKind::Completed | ConversationTurnReportKind::Failed
        )
}

fn terminal_binding_matches(
    binding: &ConversationTurnBinding,
    retained: &RetainedCommandBinding,
    command_event_id: &str,
) -> Result<bool, LedgerError> {
    let generation = u64::try_from(retained.generation).map_err(|_| LedgerError::Invalid)?;
    let context_revision =
        u64::try_from(retained.context_revision).map_err(|_| LedgerError::Invalid)?;
    Ok(binding.workspace_id == retained.workspace_id
        && binding.project_id == retained.project_id
        && binding.conversation_id == retained.conversation_id
        && binding.turn_id == retained.turn_id
        && binding.assignment_id == retained.assignment_id
        && binding.executor_generation == generation
        && binding.execution_binding_revision == retained.execution_binding_revision
        && binding.context_revision == context_revision
        && binding.command_event_id == command_event_id
        && binding.command_digest == retained.command_digest)
}

#[allow(clippy::too_many_arguments)]
fn validate_decision_binding(
    transaction: &Transaction<'_>,
    decision: &ConversationReportDecision,
    report_digest: &str,
    report_bytes: Option<&[u8]>,
    command_source: &str,
    command_event_id: &str,
    sequence: i64,
    instance_id: &str,
) -> Result<Option<Vec<u8>>, LedgerError> {
    let report_bytes = report_bytes.ok_or(LedgerError::EventConflict)?;
    let (report_identity, _, report_payload) =
        decode_report(report_bytes).map_err(|_| LedgerError::Invalid)?;
    if !decision_report_identity_matches(
        decision,
        &report_identity,
        &report_payload,
        report_digest,
        report_bytes,
        instance_id,
    ) {
        return Err(LedgerError::Invalid);
    }
    let binding = report_payload
        .binding
        .as_ref()
        .ok_or(LedgerError::Invalid)?;
    let report_kind = ConversationTurnReportKind::try_from(report_payload.kind)
        .map_err(|_| LedgerError::Invalid)?;
    let accepted = ConversationReportDecisionKind::try_from(decision.decision)
        .map_err(|_| LedgerError::Invalid)?
        == ConversationReportDecisionKind::Accepted;
    let retained = retained_decision_command(transaction, command_source, command_event_id)?;
    if accepted && retained.superseded {
        return Err(LedgerError::StaleGeneration);
    }
    if !decision_state_matches(accepted, report_kind, &decision.turn_state)
        || !sequence_matches_report_kind(sequence, report_kind)
        || !decision_scope_matches(decision, &retained)?
        || !decision_report_binding_matches(
            binding,
            &retained,
            decision,
            command_event_id,
            sequence,
            report_payload.report_sequence,
        )?
    {
        return Err(LedgerError::Invalid);
    }
    Ok((sequence == 1).then_some(report_payload.signing_public_key))
}

fn decision_report_identity_matches(
    decision: &ConversationReportDecision,
    identity: &EventIdentity,
    report: &ConversationTurnReport,
    retained_digest: &str,
    report_bytes: &[u8],
    instance_id: &str,
) -> bool {
    decision.report_source == identity.source
        && decision.report_id == identity.id
        && decision.report_digest == retained_digest
        && retained_digest == exact_digest(report_bytes)
        && report.executor_instance_id == instance_id
}

fn decision_state_matches(
    accepted: bool,
    report_kind: ConversationTurnReportKind,
    turn_state: &str,
) -> bool {
    !accepted
        || matches!(
            (report_kind, turn_state),
            (ConversationTurnReportKind::Started, "processing")
                | (ConversationTurnReportKind::Completed, "completed")
                | (ConversationTurnReportKind::Failed, "failed")
        )
}

const fn sequence_matches_report_kind(
    sequence: i64,
    report_kind: ConversationTurnReportKind,
) -> bool {
    matches!(
        (sequence, report_kind),
        (1, ConversationTurnReportKind::Started)
            | (
                2,
                ConversationTurnReportKind::Completed | ConversationTurnReportKind::Failed
            )
    )
}

fn retained_decision_command(
    transaction: &Transaction<'_>,
    command_source: &str,
    command_event_id: &str,
) -> Result<RetainedDecisionCommand, LedgerError> {
    transaction
        .query_row(
            "SELECT c.workspace_id, c.project_id, c.conversation_id, c.turn_id, c.assignment_id, \
             c.generation, EXISTS(SELECT 1 FROM commands newer \
                WHERE newer.turn_id = c.turn_id AND newer.assignment_id = c.assignment_id \
                AND newer.generation > c.generation) \
             FROM commands c WHERE c.source = ?1 AND c.event_id = ?2",
            params![command_source, command_event_id],
            |row| {
                Ok(RetainedDecisionCommand {
                    workspace_id: row.get(0)?,
                    project_id: row.get(1)?,
                    conversation_id: row.get(2)?,
                    turn_id: row.get(3)?,
                    assignment_id: row.get(4)?,
                    generation: row.get(5)?,
                    superseded: row.get(6)?,
                })
            },
        )
        .optional()?
        .ok_or(LedgerError::NotFound)
}

fn decision_scope_matches(
    decision: &ConversationReportDecision,
    retained: &RetainedDecisionCommand,
) -> Result<bool, LedgerError> {
    let generation = u64::try_from(retained.generation).map_err(|_| LedgerError::Invalid)?;
    Ok(decision.workspace_id == retained.workspace_id
        && decision.project_id == retained.project_id
        && decision.conversation_id == retained.conversation_id
        && decision.turn_id == retained.turn_id
        && decision.assignment_id == retained.assignment_id
        && decision.executor_generation == generation)
}

fn decision_report_binding_matches(
    binding: &ConversationTurnBinding,
    retained: &RetainedDecisionCommand,
    decision: &ConversationReportDecision,
    command_event_id: &str,
    sequence: i64,
    report_sequence: u64,
) -> Result<bool, LedgerError> {
    let report_sequence = i64::try_from(report_sequence).map_err(|_| LedgerError::Invalid)?;
    Ok(binding.workspace_id == retained.workspace_id
        && binding.project_id == retained.project_id
        && binding.conversation_id == retained.conversation_id
        && binding.turn_id == retained.turn_id
        && binding.assignment_id == retained.assignment_id
        && binding.executor_generation == decision.executor_generation
        && binding.command_event_id == command_event_id
        && report_sequence == sequence)
}

#[cfg(test)]
mod tests;
