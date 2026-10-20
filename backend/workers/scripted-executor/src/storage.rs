use super::{Command, LedgerError, LedgerLimits, Report, StoreOutcome};
use rusqlite::{OptionalExtension as _, Transaction, params};
use sha2::{Digest as _, Sha256};

pub fn existing_command_outcome(
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

pub fn redelivered_command_outcome(
    transaction: &Transaction<'_>,
    command: &Command<'_>,
) -> Result<StoreOutcome, LedgerError> {
    let existing: Option<(String, Option<Vec<u8>>)> = transaction
        .query_row(
            "SELECT digest, event_bytes FROM commands WHERE source = ?1 AND event_id = ?2",
            params![command.source, command.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((digest, bytes)) = existing else {
        return Err(LedgerError::NotFound);
    };
    if digest == exact_digest(command.exact_bytes)
        && bytes
            .as_deref()
            .is_none_or(|retained| retained == command.exact_bytes)
    {
        Ok(StoreOutcome::Duplicate)
    } else {
        Err(LedgerError::EventConflict)
    }
}

pub fn existing_report_outcome(
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

pub fn insert_report(
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

pub fn reserve(
    transaction: &Transaction<'_>,
    limits: LedgerLimits,
    added_records: u64,
    added_bytes: usize,
    removed_bytes: usize,
) -> Result<(), LedgerError> {
    let (records, bytes): (i64, i64) = transaction.query_row(
        "SELECT \
           (SELECT count(*) FROM commands) + (SELECT count(*) FROM reports), \
           COALESCE((SELECT sum(COALESCE(length(event_bytes), 0) + terminal_reserved_bytes) FROM commands), 0) + \
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

pub fn exact_digest(bytes: &[u8]) -> String {
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
