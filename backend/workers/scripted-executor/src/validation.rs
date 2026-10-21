use super::{Command, LedgerError, Report};
use ayni_orchestrator_events::v1::{ConversationReportDecision, ConversationReportDecisionKind};
use ayni_orchestrator_events::workflow::{decode_command, decode_decision};
use ed25519_dalek::SigningKey;
use rusqlite::{Connection, Transaction, params};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn unix_seconds() -> Result<u64, LedgerError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|_| LedgerError::Invalid)
}
pub(super) fn validate_redelivered_command(command: &Command<'_>) -> Result<(), LedgerError> {
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
pub(super) const fn validate_command(command: &Command<'_>) -> Result<(), LedgerError> {
    if command.source.is_empty()
        || command.id.is_empty()
        || command.exact_bytes.is_empty()
        || command.turn_id.is_empty()
        || command.assignment_id.is_empty()
        || command.executor_generation == 0
    {
        Err(LedgerError::Invalid)
    } else {
        Ok(())
    }
}
pub(super) const fn validate_report(report: &Report<'_>, sequence: u8) -> Result<(), LedgerError> {
    if report.source.is_empty()
        || report.id.is_empty()
        || report.exact_bytes.is_empty()
        || report.sequence != sequence
    {
        Err(LedgerError::Invalid)
    } else {
        Ok(())
    }
}
pub(super) fn retained_instance_id(connection: &Connection) -> Result<String, LedgerError> {
    Ok(connection.query_row(
        "SELECT instance_id FROM executor_identity WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?)
}
pub(super) fn retained_signing_public_key(
    connection: &Connection,
) -> Result<[u8; 32], LedgerError> {
    let seed: Vec<u8> = connection.query_row(
        "SELECT signing_seed FROM executor_identity WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    let seed: [u8; 32] = seed.try_into().map_err(|_| LedgerError::Invalid)?;
    Ok(SigningKey::from_bytes(&seed).verifying_key().to_bytes())
}
pub(super) fn decode_targeted_decision(
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
    match ConversationReportDecisionKind::try_from(payload.decision)
        .map_err(|_| LedgerError::Invalid)?
    {
        ConversationReportDecisionKind::Accepted => Ok((payload, true)),
        ConversationReportDecisionKind::Rejected => Ok((payload, false)),
        ConversationReportDecisionKind::Unspecified => Err(LedgerError::Invalid),
    }
}
pub(super) fn retain_decision_consumer_sequence(
    transaction: &Transaction<'_>,
    report_source: &str,
    report_event_id: &str,
    consumer_sequence: Option<u64>,
) -> Result<(), LedgerError> {
    let Some(sequence) = consumer_sequence else {
        return Ok(());
    };
    transaction.execute("UPDATE reports SET decision_consumer_sequence = CASE WHEN COALESCE(decision_consumer_sequence, 0) < ?3 THEN ?3 ELSE decision_consumer_sequence END WHERE source = ?1 AND event_id = ?2", params![report_source, report_event_id, i64::try_from(sequence).map_err(|_| LedgerError::Invalid)?])?;
    Ok(())
}
