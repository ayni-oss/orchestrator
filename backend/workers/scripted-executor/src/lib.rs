//! Retained local ledger prototype for the deterministic M1 executor.
//!
//! The ledger owns exact command and report bytes until the corresponding
//! application decision is durable. It is deliberately synchronous; a future
//! worker process must call it from a blocking boundary.

use ayni_orchestrator_events::v1::{
    ConversationReportDecision, ConversationReportDecisionKind, ConversationTurnReportKind,
};
use ayni_orchestrator_events::workflow::{
    decode_command, decode_decision, decode_report, verify_report_signature,
};
use rusqlite::{Connection, OptionalExtension as _, Transaction, TransactionBehavior, params};
use sha2::{Digest as _, Sha256};
use std::path::Path;

const SCHEMA: &str = r"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS executor_identity (
    singleton integer PRIMARY KEY CHECK (singleton = 1),
    instance_id text NOT NULL,
    signing_seed blob NOT NULL CHECK (length(signing_seed) = 32)
);

CREATE TABLE IF NOT EXISTS commands (
    source text NOT NULL,
    event_id text NOT NULL,
    digest text NOT NULL,
    event_bytes blob,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    turn_id text NOT NULL,
    assignment_id text NOT NULL,
    generation integer NOT NULL CHECK (generation > 0),
    execution_binding_revision text NOT NULL,
    context_revision integer NOT NULL CHECK (context_revision > 0),
    signing_public_key blob CHECK (signing_public_key IS NULL OR length(signing_public_key) = 32),
    state text NOT NULL CHECK (state IN ('claim_pending', 'accepted', 'terminal_pending', 'rejected', 'completed')),
    PRIMARY KEY (source, event_id),
    UNIQUE (turn_id, assignment_id, generation)
);

CREATE TABLE IF NOT EXISTS reports (
    source text NOT NULL,
    event_id text NOT NULL,
    digest text NOT NULL,
    event_bytes blob,
    command_source text NOT NULL,
    command_event_id text NOT NULL,
    sequence integer NOT NULL CHECK (sequence IN (1, 2)),
    decision text CHECK (decision IN ('accepted', 'rejected')),
    decision_digest text,
    decision_bytes blob,
    decision_acked integer NOT NULL DEFAULT 0 CHECK (decision_acked IN (0, 1)),
    PRIMARY KEY (source, event_id),
    UNIQUE (command_source, command_event_id, sequence),
    FOREIGN KEY (command_source, command_event_id) REFERENCES commands(source, event_id)
);
";

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
    #[error("executor ledger capacity is exhausted")]
    Capacity,
    #[error("the report or command does not exist")]
    NotFound,
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

type DecisionRow = (
    Option<String>,
    Option<String>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    String,
    i64,
    String,
    String,
);

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
        let validated = validate_command_claim(command, claim, &instance_id)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(outcome) = existing_command_outcome(&transaction, command, claim)? {
            transaction.commit()?;
            return Ok(outcome);
        }
        let latest_generation: Option<i64> = transaction
            .query_row(
                "SELECT max(generation) FROM commands WHERE turn_id = ?1 AND assignment_id = ?2",
                params![validated.turn_id, validated.assignment_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let generation = i64::try_from(validated.generation).map_err(|_| LedgerError::Invalid)?;
        if latest_generation.is_some_and(|latest| generation < latest) {
            return Err(LedgerError::StaleGeneration);
        }
        if latest_generation == Some(generation) {
            return Err(LedgerError::EventConflict);
        }
        transaction.execute(
            "UPDATE commands SET state = 'rejected' \
             WHERE turn_id = ?1 AND assignment_id = ?2 AND generation < ?3 \
             AND state != 'completed'",
            params![validated.turn_id, validated.assignment_id, generation],
        )?;
        reserve(
            &transaction,
            self.limits,
            2,
            command.exact_bytes.len() + claim.exact_bytes.len(),
            0,
        )?;
        transaction.execute(
            "INSERT INTO commands \
             (source, event_id, digest, event_bytes, workspace_id, project_id, conversation_id, \
              turn_id, assignment_id, generation, execution_binding_revision, context_revision, state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'claim_pending')",
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
            ],
        )?;
        insert_report(&transaction, command, claim)?;
        transaction.commit()?;
        Ok(StoreOutcome::Stored)
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
        if let Some(outcome) = existing_report_outcome(&transaction, report)? {
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
        reserve(&transaction, self.limits, 1, report.exact_bytes.len(), 0)?;
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
            "UPDATE commands SET state = 'terminal_pending' WHERE source = ?1 AND event_id = ?2",
            params![command_source, command_event_id],
        )?;
        transaction.commit()?;
        Ok(StoreOutcome::Stored)
    }

    /// Record an application decision before acknowledging its consumer delivery.
    /// Exact report bytes are removed only after this transaction commits.
    ///
    /// # Errors
    /// Returns missing reports, immutable conflicts, capacity, or SQL errors.
    #[allow(clippy::too_many_lines)]
    pub fn record_decision(
        &mut self,
        decision: &Decision<'_>,
    ) -> Result<StoreOutcome, LedgerError> {
        if decision.exact_bytes.is_empty() {
            return Err(LedgerError::Invalid);
        }
        let (_, payload) =
            decode_decision(decision.exact_bytes).map_err(|_| LedgerError::Invalid)?;
        let accepted = match ConversationReportDecisionKind::try_from(payload.decision)
            .map_err(|_| LedgerError::Invalid)?
        {
            ConversationReportDecisionKind::Accepted => true,
            ConversationReportDecisionKind::Rejected => false,
            ConversationReportDecisionKind::Unspecified => return Err(LedgerError::Invalid),
        };
        let instance_id = retained_instance_id(&self.connection)?;
        if payload.target_executor_instance_id != instance_id {
            return Err(LedgerError::Invalid);
        }
        let decision_digest = exact_digest(decision.exact_bytes);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<DecisionRow> = transaction
            .query_row(
                "SELECT decision, decision_digest, decision_bytes, event_bytes, digest, sequence, command_source, command_event_id \
                 FROM reports WHERE source = ?1 AND event_id = ?2",
                params![payload.report_source, payload.report_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            prior_result,
            prior_digest,
            prior_bytes,
            report_bytes,
            report_digest,
            sequence,
            command_source,
            command_id,
        )) = existing
        else {
            return Err(LedgerError::NotFound);
        };
        let result = if accepted { "accepted" } else { "rejected" };
        if let Some(prior_result) = prior_result {
            if prior_result == result
                && prior_digest.as_deref() == Some(decision_digest.as_str())
                && prior_bytes
                    .as_deref()
                    .is_none_or(|bytes| bytes == decision.exact_bytes)
            {
                transaction.commit()?;
                return Ok(StoreOutcome::Duplicate);
            }
            return Err(LedgerError::EventConflict);
        }
        let signing_public_key = validate_decision_binding(
            &transaction,
            &payload,
            &report_digest,
            report_bytes.as_deref(),
            &command_source,
            &command_id,
            sequence,
            &instance_id,
        )?;
        reserve(
            &transaction,
            self.limits,
            0,
            decision.exact_bytes.len(),
            report_bytes.as_deref().map_or(0, <[u8]>::len),
        )?;
        transaction.execute(
            "UPDATE reports SET decision = ?3, decision_digest = ?4, decision_bytes = ?5, event_bytes = NULL \
             WHERE source = ?1 AND event_id = ?2",
            params![
                payload.report_source,
                payload.report_id,
                result,
                decision_digest,
                decision.exact_bytes,
            ],
        )?;
        let command_state = match (sequence, accepted) {
            (1, true) => "accepted",
            (1, false) => "rejected",
            (2, false) => "terminal_pending",
            (2, true) => "completed",
            _ => return Err(LedgerError::Invalid),
        };
        let cleanup_command = (sequence == 1 && !accepted) || (sequence == 2 && accepted);
        transaction.execute(
            "UPDATE commands SET state = ?3, signing_public_key = COALESCE(?5, signing_public_key), \
             event_bytes = CASE WHEN ?4 THEN NULL ELSE event_bytes END \
             WHERE source = ?1 AND event_id = ?2",
            params![
                command_source,
                command_id,
                command_state,
                cleanup_command,
                signing_public_key,
            ],
        )?;
        transaction.commit()?;
        Ok(StoreOutcome::Stored)
    }

    /// Mark a decision-consumer ACK only after its durable ledger transaction.
    /// The exact decision bytes may then be removed while identity/digest remain.
    ///
    /// # Errors
    /// Returns missing decisions or SQL failures.
    pub fn acknowledge_decision(
        &mut self,
        report_source: &str,
        report_event_id: &str,
    ) -> Result<(), LedgerError> {
        let changed = self.connection.execute(
            "UPDATE reports SET decision_acked = 1, decision_bytes = NULL \
             WHERE source = ?1 AND event_id = ?2 AND decision IS NOT NULL",
            params![report_source, report_event_id],
        )?;
        if changed == 0 {
            return Err(LedgerError::NotFound);
        }
        Ok(())
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

fn validate_command_claim(
    command: &Command<'_>,
    claim: &Report<'_>,
    instance_id: &str,
) -> Result<ValidatedCommand, LedgerError> {
    let (command_identity, command_payload) =
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
    let (claim_identity, claim_payload) =
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
    type CommandBindingRow = (
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        String,
        i64,
        Vec<u8>,
        String,
    );
    let retained: Option<CommandBindingRow> = transaction
        .query_row(
            "SELECT digest, workspace_id, project_id, conversation_id, turn_id, assignment_id, \
             generation, execution_binding_revision, context_revision, signing_public_key, state \
             FROM commands WHERE source = ?1 AND event_id = ?2",
            params![command_source, command_event_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                ))
            },
        )
        .optional()?;
    let Some((
        command_digest,
        workspace_id,
        project_id,
        conversation_id,
        turn_id,
        assignment_id,
        generation,
        execution_binding_revision,
        context_revision,
        public_key,
        state,
    )) = retained
    else {
        return Err(LedgerError::NotFound);
    };
    if state != "accepted" {
        return Err(LedgerError::ClaimNotAccepted);
    }
    let (identity, payload) =
        decode_report(report.exact_bytes).map_err(|_| LedgerError::Invalid)?;
    let binding = payload.binding.as_ref().ok_or(LedgerError::Invalid)?;
    let kind =
        ConversationTurnReportKind::try_from(payload.kind).map_err(|_| LedgerError::Invalid)?;
    let public_key: [u8; 32] = public_key.try_into().map_err(|_| LedgerError::Invalid)?;
    let report_identity_matches = identity.source == report.source && identity.id == report.id;
    if !report_identity_matches
        || payload.executor_instance_id != instance_id
        || payload.report_sequence != 2
        || !matches!(
            kind,
            ConversationTurnReportKind::Completed | ConversationTurnReportKind::Failed
        )
        || binding.workspace_id != workspace_id
        || binding.project_id != project_id
        || binding.conversation_id != conversation_id
        || binding.turn_id != turn_id
        || binding.assignment_id != assignment_id
        || binding.executor_generation
            != u64::try_from(generation).map_err(|_| LedgerError::Invalid)?
        || binding.execution_binding_revision != execution_binding_revision
        || binding.context_revision
            != u64::try_from(context_revision).map_err(|_| LedgerError::Invalid)?
        || binding.command_event_id != command_event_id
        || binding.command_digest != command_digest
    {
        return Err(LedgerError::Invalid);
    }
    verify_report_signature(&identity, &payload, Some(&public_key))
        .map_err(|_| LedgerError::Invalid)
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
    let (report_identity, report_payload) =
        decode_report(report_bytes).map_err(|_| LedgerError::Invalid)?;
    if decision.report_source != report_identity.source
        || decision.report_id != report_identity.id
        || decision.report_digest != report_digest
        || report_digest != exact_digest(report_bytes)
        || report_payload.executor_instance_id != instance_id
    {
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
    let accepted_state_matches = !accepted
        || matches!(
            (report_kind, decision.turn_state.as_str()),
            (ConversationTurnReportKind::Started, "processing")
                | (ConversationTurnReportKind::Completed, "completed")
                | (ConversationTurnReportKind::Failed, "failed")
        );
    let sequence_matches_kind = matches!(
        (sequence, report_kind),
        (1, ConversationTurnReportKind::Started)
            | (
                2,
                ConversationTurnReportKind::Completed | ConversationTurnReportKind::Failed
            )
    );
    let retained: Option<(String, String, String, String, String, i64, bool)> = transaction
        .query_row(
            "SELECT c.workspace_id, c.project_id, c.conversation_id, c.turn_id, c.assignment_id, \
             c.generation, EXISTS(SELECT 1 FROM commands newer \
                WHERE newer.turn_id = c.turn_id AND newer.assignment_id = c.assignment_id \
                AND newer.generation > c.generation) \
             FROM commands c WHERE c.source = ?1 AND c.event_id = ?2",
            params![command_source, command_event_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    let Some((workspace, project, conversation, turn, assignment, generation, superseded)) =
        retained
    else {
        return Err(LedgerError::NotFound);
    };
    if accepted && superseded {
        return Err(LedgerError::StaleGeneration);
    }
    if !accepted_state_matches
        || !sequence_matches_kind
        || decision.workspace_id != workspace
        || decision.project_id != project
        || decision.conversation_id != conversation
        || decision.turn_id != turn
        || decision.assignment_id != assignment
        || decision.executor_generation
            != u64::try_from(generation).map_err(|_| LedgerError::Invalid)?
        || binding.workspace_id != workspace
        || binding.project_id != project
        || binding.conversation_id != conversation
        || binding.turn_id != turn
        || binding.assignment_id != assignment
        || binding.executor_generation != decision.executor_generation
        || binding.command_event_id != command_event_id
        || i64::try_from(report_payload.report_sequence).map_err(|_| LedgerError::Invalid)?
            != sequence
    {
        return Err(LedgerError::Invalid);
    }
    if sequence == 1 {
        Ok(Some(report_payload.signing_public_key))
    } else {
        Ok(None)
    }
}

fn existing_command_outcome(
    transaction: &Transaction<'_>,
    command: &Command<'_>,
    claim: &Report<'_>,
) -> Result<Option<StoreOutcome>, LedgerError> {
    let existing: Option<(String, Option<Vec<u8>>)> = transaction
        .query_row(
            "SELECT digest, event_bytes FROM commands WHERE source = ?1 AND event_id = ?2",
            params![command.source, command.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((digest, bytes)) = existing else {
        return Ok(None);
    };
    if digest != exact_digest(command.exact_bytes)
        || bytes
            .as_deref()
            .is_some_and(|retained| retained != command.exact_bytes)
    {
        return Err(LedgerError::EventConflict);
    }
    match existing_report_outcome(transaction, claim)? {
        Some(StoreOutcome::Duplicate) => Ok(Some(StoreOutcome::Duplicate)),
        Some(StoreOutcome::Stored) | None => Err(LedgerError::EventConflict),
    }
}

fn existing_report_outcome(
    transaction: &Transaction<'_>,
    report: &Report<'_>,
) -> Result<Option<StoreOutcome>, LedgerError> {
    let existing: Option<(String, Option<Vec<u8>>)> = transaction
        .query_row(
            "SELECT digest, event_bytes FROM reports WHERE source = ?1 AND event_id = ?2",
            params![report.source, report.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((digest, bytes)) = existing else {
        return Ok(None);
    };
    if digest == exact_digest(report.exact_bytes)
        && bytes
            .as_deref()
            .is_none_or(|retained| retained == report.exact_bytes)
    {
        Ok(Some(StoreOutcome::Duplicate))
    } else {
        Err(LedgerError::EventConflict)
    }
}

fn insert_report(
    transaction: &Transaction<'_>,
    command: &Command<'_>,
    report: &Report<'_>,
) -> Result<(), LedgerError> {
    transaction.execute(
        "INSERT INTO reports \
         (source, event_id, digest, event_bytes, command_source, command_event_id, sequence) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            report.source,
            report.id,
            exact_digest(report.exact_bytes),
            report.exact_bytes,
            command.source,
            command.id,
            i64::from(report.sequence),
        ],
    )?;
    Ok(())
}

fn reserve(
    transaction: &Transaction<'_>,
    limits: LedgerLimits,
    added_records: u64,
    added_bytes: usize,
    removed_bytes: usize,
) -> Result<(), LedgerError> {
    let (records, bytes): (i64, i64) = transaction.query_row(
        "SELECT \
           (SELECT count(*) FROM commands) + (SELECT count(*) FROM reports), \
           COALESCE((SELECT sum(length(event_bytes)) FROM commands), 0) + \
           COALESCE((SELECT sum(COALESCE(length(event_bytes), 0) + COALESCE(length(decision_bytes), 0)) FROM reports), 0)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let added_bytes = u64::try_from(added_bytes).map_err(|_| LedgerError::Capacity)?;
    let removed_bytes = u64::try_from(removed_bytes).map_err(|_| LedgerError::Capacity)?;
    if u64::try_from(records)
        .map_err(|_| LedgerError::Capacity)?
        .saturating_add(added_records)
        > limits.max_records
        || u64::try_from(bytes)
            .map_err(|_| LedgerError::Capacity)?
            .saturating_sub(removed_bytes)
            .saturating_add(added_bytes)
            > limits.max_exact_bytes
    {
        return Err(LedgerError::Capacity);
    }
    Ok(())
}

fn exact_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut result = String::with_capacity(7 + digest.len() * 2);
    result.push_str("sha256:");
    for byte in digest {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_orchestrator_events::EventIdentity;
    use ayni_orchestrator_events::v1::{
        ConversationReportDecision, ConversationReportDecisionKind, ConversationTurnBinding,
        ConversationTurnReport, ConversationTurnReportKind, ConversationTurnRequested,
    };
    use ayni_orchestrator_events::workflow::{
        APPLICATION_SOURCE, decode_report, encode_command, encode_decision, encode_report,
        sign_report, signing_key_digest,
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
        let bytes = encode_command(&identity, &payload).map_err(|_| LedgerError::Invalid)?;
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
        let bytes = encode_report(&identity, &payload).map_err(|_| LedgerError::Invalid)?;
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
        let bytes = encode_report(&identity, &payload).map_err(|_| LedgerError::Invalid)?;
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
        let (_, report_payload) = decode_report(&report.bytes).map_err(|_| LedgerError::Invalid)?;
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
            id: "018f0f7b-6a4d-7a11-8d77-b1b2b3b4b5b6".into(),
        };
        encode_decision(&identity, &payload).map_err(|_| LedgerError::Invalid)
    }

    #[test]
    fn crash_reopen_preserves_claim_and_blocks_execution_until_decision() -> Result<(), LedgerError>
    {
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
        let terminal_decision =
            decision_bytes(&command, &terminal, WINNER, WINNER, true, None, None)?;
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
        let loser_decision =
            decision_bytes(&command, &loser_claim, LOSER, WINNER, false, None, None)?;
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
    fn newer_generation_fences_prior_acceptance() -> Result<(), LedgerError> {
        let directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
        let mut ledger = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
        let generation_one = command_fixture(1, COMMAND_ID)?;
        let claim_one = claim_fixture(&generation_one, WINNER, &WINNER_SEED, CLAIM_ID)?;
        ledger.retain_command_and_claim(&generation_one.command(), &claim_one.report(1))?;
        let decision_one = decision_bytes(
            &generation_one,
            &claim_one,
            WINNER,
            WINNER,
            true,
            None,
            None,
        )?;
        ledger.record_decision(&Decision {
            exact_bytes: &decision_one,
        })?;
        assert!(ledger.can_execute(&generation_one.identity.source, &generation_one.identity.id)?);

        let generation_two = command_fixture(2, "018f0f7b-6a4d-7a11-8d77-d1d2d3d4d5d6")?;
        let claim_two = claim_fixture(
            &generation_two,
            WINNER,
            &WINNER_SEED,
            "018f0f7b-6a4d-7a11-8d77-e1e2e3e4e5e6",
        )?;
        ledger.retain_command_and_claim(&generation_two.command(), &claim_two.report(1))?;
        assert!(!ledger.can_execute(&generation_one.identity.source, &generation_one.identity.id)?);
        assert_eq!(
            ledger.record_decision(&Decision {
                exact_bytes: &decision_one,
            })?,
            StoreOutcome::Duplicate
        );
        assert!(!ledger.can_execute(&generation_one.identity.source, &generation_one.identity.id)?);

        let decision_two = decision_bytes(
            &generation_two,
            &claim_two,
            WINNER,
            WINNER,
            true,
            None,
            None,
        )?;
        ledger.record_decision(&Decision {
            exact_bytes: &decision_two,
        })?;
        assert!(ledger.can_execute(&generation_two.identity.source, &generation_two.identity.id)?);

        let late_directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
        let mut late = open(
            &late_directory,
            LedgerLimits::m1_default(),
            WINNER,
            &WINNER_SEED,
        )?;
        late.retain_command_and_claim(&generation_one.command(), &claim_one.report(1))?;
        late.retain_command_and_claim(&generation_two.command(), &claim_two.report(1))?;
        assert!(matches!(
            late.record_decision(&Decision {
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
        let changed_command_claim =
            claim_fixture(&changed_command, WINNER, &WINNER_SEED, CLAIM_ID)?;
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
            Err(LedgerError::EventConflict)
        ));

        let accepted = decision_bytes(&command, &claim, WINNER, WINNER, true, None, None)?;
        ledger.record_decision(&Decision {
            exact_bytes: &accepted,
        })?;
        let (report_bytes, retained_decision_bytes): (Option<Vec<u8>>, Option<Vec<u8>>) = ledger
            .connection
            .query_row(
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
        let mut full = open(
            &full_directory,
            LedgerLimits {
                max_records: 1,
                max_exact_bytes: 4,
            },
            WINNER,
            &WINNER_SEED,
        )?;
        let command = command_fixture(1, COMMAND_ID)?;
        let claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
        assert!(matches!(
            full.retain_command_and_claim(&command.command(), &claim.report(1)),
            Err(LedgerError::Capacity)
        ));
        assert!(!full.can_execute(&command.identity.source, &command.identity.id)?);
        Ok(())
    }
}
