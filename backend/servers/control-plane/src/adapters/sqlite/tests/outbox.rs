use super::super::verification_outbox::HISTORY_SECONDS;
use super::verification::{accepted, count, prepared};
use super::*;
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn publication_retries_survive_restart_without_mutating_accepted_state() -> TestResult {
    let dir = tempdir()?;
    let path = dir.path().join("outbox.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (profile, cursor, report, command) = prepared(&mut store)?;
    store.commit_verification(
        &command,
        &serde_json::to_string(&cursor)?,
        &accepted(&cursor, &report)?,
    )?;
    let lifecycle_pending: i64 = store.connection.query_row(
        "SELECT COUNT(*) FROM lifecycle_outbox WHERE stored_at IS NULL",
        [],
        |row| row.get(0),
    )?;
    let before = store.runtime_profile(&profile)?;
    let watermark = store.event_watermark()?;
    let now = unix_seconds()?;
    let publications = store.claim_publications(now)?;
    assert_eq!(publications.len(), 1);
    assert!(store.claim_publications(now)?.is_empty());
    store.record_publication(&publications[0], None)?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    assert!(store.claim_publications(now + 1)?.is_empty());
    assert_eq!(store.claim_publications(now + 2)?, publications);
    // A broker ACK followed by a crash before the local write retries the same bytes.
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(store.claim_publications(now + 6)?, publications);
    store.record_publication(&publications[0], Some(7))?;
    assert!(store.claim_publications(i64::MAX - 300)?.is_empty());
    assert_eq!(store.outbox_stats()?.pending, lifecycle_pending);
    assert_eq!(store.outbox_stats()?.retained, 1);
    let (attempts, sequence): (i64, i64) = store.connection.query_row(
        "SELECT attempts,broker_sequence FROM verification_outbox",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    assert_eq!((attempts, sequence), (3, 7));
    assert_eq!(store.runtime_profile(&profile)?, before);
    assert_eq!(store.event_watermark()?, watermark);
    drop(store);
    CleanStore::open(path, 3)?;
    Ok(())
}

#[test]
fn retention_keeps_unpublished_and_active_cleanup_and_rejects_pruned_replay() -> TestResult {
    let mut store = CleanStore::in_memory()?;
    let (profile, cursor, report, command) = prepared(&mut store)?;
    let expected = serde_json::to_string(&cursor)?;
    let next = accepted(&cursor, &report)?;
    store.commit_verification(&command, &expected, &next)?;
    let future = unix_seconds()? + HISTORY_SECONDS + 1;
    store.prune_verification_history_at(future)?;
    assert_eq!(count(&store, "verification_journal")?, 1);
    let publication = store.claim_publications(unix_seconds()?)?.remove(0);
    store.record_publication(&publication, Some(1))?;
    store.prune_verification_history_at(future)?;
    assert_eq!(count(&store, "verification_journal")?, 1);
    // The drained old generation remains replay-safe until its retention expires.
    let mut drained = next.clone();
    if std::env::var("AYNI_CONSUMER_FIXTURE").as_deref() == Ok("true") {
        drained.next_at = u64::MAX;
    } else {
        drained.pending = None;
        drained.preparation = None;
    }
    store.save_runtime_cursor(&profile, &drained)?;
    store.prune_verification_history_at(unix_seconds()?)?;
    assert_eq!(count(&store, "verification_journal")?, 1);
    let before = store.runtime_profile(&profile)?;
    let watermark = store.event_watermark()?;
    store.prune_verification_history_at(future)?;
    assert_eq!(count(&store, "verification_journal")?, 0);
    assert_eq!(count(&store, "verification_outbox")?, 0);
    assert!(
        store
            .commit_verification(&command, &expected, &next)
            .is_err()
    );
    assert_eq!(store.runtime_profile(&profile)?, before);
    assert_eq!(store.event_watermark()?, watermark);
    Ok(())
}

#[test]
fn schema_thirteen_migration_keeps_immutable_pending_bytes() -> TestResult {
    let dir = tempdir()?;
    let path = dir.path().join("migration.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (_, cursor, report, command) = prepared(&mut store)?;
    store.commit_verification(
        &command,
        &serde_json::to_string(&cursor)?,
        &accepted(&cursor, &report)?,
    )?;
    let pending = store.unpublished_verification_events()?;
    store.connection.execute_batch("DROP TABLE project_context_summaries; DROP TABLE project_context_records; DROP TABLE workspaces; DROP TABLE project_conversation_actions; DROP TABLE project_conversations; DROP TABLE project_agent_requests; DROP TABLE project_outcome_summaries; DROP TABLE lifecycle_outbox; DROP TABLE lifecycle_journal; DROP TABLE coding_inbox; DROP TABLE coding_executions; DROP TABLE runtime_operation_inbox; DROP TABLE verification_consumer; DROP INDEX verification_outbox_pending; DROP INDEX verification_outbox_stored; ALTER TABLE verification_outbox DROP COLUMN next_attempt_at; ALTER TABLE verification_outbox DROP COLUMN broker_sequence; ALTER TABLE verification_outbox DROP COLUMN last_error; UPDATE clean_schema SET version=13;")?;
    drop(store);
    let mut store = CleanStore::open(path, 3)?;
    assert_eq!(store.schema_version()?, SCHEMA_VERSION);
    assert_eq!(store.unpublished_verification_events()?, pending);
    assert_eq!(store.claim_publications(0)?.len(), 1);
    Ok(())
}

#[test]
#[ignore = "exports synthetic accepted state for the disposable local publisher fixture"]
fn export_outbox_fixture() -> TestResult {
    let directory = std::path::PathBuf::from(std::env::var("AYNI_OUTBOX_FIXTURE_DIRECTORY")?);
    let mut store = CleanStore::open(directory.join("orchestrator.sqlite"), 3)?;
    let (_, cursor, report, command) = prepared(&mut store)?;
    store.commit_verification(
        &command,
        &serde_json::to_string(&cursor)?,
        &accepted(&cursor, &report)?,
    )?;
    // The fixture contains an accepted decision, with no live provider or runtime work.
    let mut drained = accepted(&cursor, &report)?;
    if std::env::var("AYNI_CONSUMER_FIXTURE").as_deref() == Ok("true") {
        drained.next_at = u64::MAX;
    } else {
        drained.pending = None;
        drained.preparation = None;
    }
    store.save_runtime_cursor(&command.profile_id, &drained)?;
    let repository = store.repository_connection(
        &store
            .runtime_profile(&command.profile_id)?
            .repository_connection_id,
    )?;
    store.transition_repository_connection(
        &repository.id,
        repository.resource_version,
        v1::RepositoryConnectionStatus::Connected,
        None,
    )?;
    store.record_onboarding(&repository.id, "ready", "", SECOND_REVISION)?;
    let publication = store.claim_publications(unix_seconds()?)?.remove(0);
    std::fs::write(directory.join("event.json"), &publication.bytes)?;
    std::fs::write(
        directory.join("message-id"),
        crate::coordinators::outbox::message_id(&publication.identity),
    )?;
    Ok(())
}

#[test]
fn bounded_batches_are_fair_and_capacity_counts_only_pending_delivery() -> TestResult {
    let mut store = CleanStore::in_memory()?;
    let (_, cursor, report, command) = prepared(&mut store)?;
    store.commit_verification(
        &command,
        &serde_json::to_string(&cursor)?,
        &accepted(&cursor, &report)?,
    )?;
    // Populate only persistence bookkeeping for the admission/batching boundary.
    store.connection.execute_batch("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<99995)
INSERT INTO verification_journal SELECT 'fixture-'||i,profile_id,preparation_id,'fixture-'||i,report_sha256,aggregate_revision,payload,created_at FROM n,verification_journal WHERE id NOT LIKE 'fixture-%';
INSERT INTO verification_outbox(journal_id,source,event_id,payload) SELECT id,'fixture',id,x'00' FROM verification_journal WHERE id LIKE 'fixture-%';")?;
    assert!(matches!(
        store.admit_outbox_capacity(),
        Err(CleanStoreError::BoundExceeded)
    ));
    let first = store.claim_publications(100)?;
    let second = store.claim_publications(100)?;
    assert_eq!((first.len(), second.len()), (8, 8));
    assert!(first.iter().all(|p| !second.contains(p)));
    store.record_publication(&first[0], Some(1))?;
    store.admit_outbox_capacity()?;
    assert_eq!(store.outbox_stats()?.pending, 99999);
    Ok(())
}
