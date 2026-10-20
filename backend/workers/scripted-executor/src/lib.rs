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
use rusqlite::{Connection, OptionalExtension as _, Transaction, TransactionBehavior, params};
use sha2::{Digest as _, Sha256};
use std::path::Path;

mod schema;

use schema::SCHEMA;

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

struct RetainedDecisionReport {
    prior_result: Option<String>,
    prior_digest: Option<String>,
    prior_bytes: Option<Vec<u8>>,
    report_bytes: Option<Vec<u8>>,
    report_digest: String,
    sequence: i64,
    command_source: String,
    command_event_id: String,
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
    pub fn record_decision(
        &mut self,
        decision: &Decision<'_>,
    ) -> Result<StoreOutcome, LedgerError> {
        let instance_id = retained_instance_id(&self.connection)?;
        let (payload, accepted) = decode_targeted_decision(decision.exact_bytes, &instance_id)?;
        let decision_digest = exact_digest(decision.exact_bytes);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let retained = retained_decision_report(&transaction, &payload)?;
        let result = if accepted { "accepted" } else { "rejected" };
        if prior_decision_is_duplicate(&retained, result, &decision_digest, decision.exact_bytes)? {
            transaction.commit()?;
            return Ok(StoreOutcome::Duplicate);
        }
        let signing_public_key = validate_decision_binding(
            &transaction,
            &payload,
            &retained.report_digest,
            retained.report_bytes.as_deref(),
            &retained.command_source,
            &retained.command_event_id,
            retained.sequence,
            &instance_id,
        )?;
        reserve(
            &transaction,
            self.limits,
            0,
            decision.exact_bytes.len(),
            retained.report_bytes.as_deref().map_or(0, <[u8]>::len),
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
        let (command_state, cleanup_command) = command_transition(retained.sequence, accepted)?;
        transaction.execute(
            "UPDATE commands SET state = ?3, signing_public_key = COALESCE(?5, signing_public_key), \
             event_bytes = CASE WHEN ?4 THEN NULL ELSE event_bytes END \
             WHERE source = ?1 AND event_id = ?2",
            params![
                retained.command_source,
                retained.command_event_id,
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

fn retained_decision_report(
    transaction: &Transaction<'_>,
    decision: &ConversationReportDecision,
) -> Result<RetainedDecisionReport, LedgerError> {
    transaction
        .query_row(
            "SELECT decision, decision_digest, decision_bytes, event_bytes, digest, sequence, command_source, command_event_id \
             FROM reports WHERE source = ?1 AND event_id = ?2",
            params![decision.report_source, decision.report_id],
            |row| {
                Ok(RetainedDecisionReport {
                    prior_result: row.get(0)?,
                    prior_digest: row.get(1)?,
                    prior_bytes: row.get(2)?,
                    report_bytes: row.get(3)?,
                    report_digest: row.get(4)?,
                    sequence: row.get(5)?,
                    command_source: row.get(6)?,
                    command_event_id: row.get(7)?,
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
        (1, false) => Ok(("rejected", true)),
        (2, false) => Ok(("terminal_pending", false)),
        (2, true) => Ok(("completed", true)),
        _ => Err(LedgerError::Invalid),
    }
}

fn validate_command_claim(
    command: &Command<'_>,
    claim: &Report<'_>,
    instance_id: &str,
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
mod tests;
