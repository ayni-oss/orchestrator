//! Real `SQLite` commit, replay, generation and bounded rejection invariants.
use super::verification::{accepted, count, prepared};
use super::*;
type TestResult = Result<(), Box<dyn std::error::Error>>;
use ayni_orchestrator_events::decisions::{decode, encode};

fn seed(store: &mut CleanStore) -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
    let (profile, cursor, report, command) = prepared(store)?;
    let mut next = accepted(&cursor, &report)?;
    next.next_at = u64::MAX;
    store.commit_verification(&command, &serde_json::to_string(&cursor)?, &next)?;
    let bytes = store
        .connection
        .query_row("SELECT payload FROM verification_outbox", [], |r| r.get(0))?;
    Ok((profile, bytes))
}

#[test]
fn commit_wakes_only_accepted_cleanup_and_restart_replay_preserves_budgets() -> TestResult {
    let dir = tempdir()?;
    let path = dir.path().join("consumer.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (profile, bytes) = seed(&mut store)?;
    let watermark = store.event_watermark()?;
    let before = store.runtime_cursor(&profile)?;
    assert_eq!(store.consume_verification(&bytes)?, "woken");
    let mut expected = before.clone();
    expected.next_at = 0;
    assert_eq!(
        serde_json::to_value(store.runtime_cursor(&profile)?)?,
        serde_json::to_value(expected)?
    );
    assert_eq!(store.event_watermark()?, watermark);
    // Model a committed receipt whose broker ACK was lost, followed by later backoff.
    let mut later = before;
    later.next_at = 123;
    store.save_runtime_cursor(&profile, &later)?;
    drop(store);
    let mut store = CleanStore::open(path, 3)?;
    let watermark = store.event_watermark()?;
    assert_eq!(store.consume_verification(&bytes)?, "duplicate");
    assert_eq!(store.runtime_cursor(&profile)?.next_at, 123);
    assert_eq!(store.event_watermark()?, watermark);
    assert_eq!(count(&store, "verification_consumer")?, 1);
    Ok(())
}

#[test]
fn stale_generation_and_drained_cleanup_are_receipted_without_mutation() -> TestResult {
    let mut store = CleanStore::in_memory()?;
    let (profile, bytes) = seed(&mut store)?;
    let mut cursor = store.runtime_cursor(&profile)?;
    cursor.preparation.as_mut().ok_or("preparation")?.generation = "replacement".into();
    store.save_runtime_cursor(&profile, &cursor)?;
    assert_eq!(store.consume_verification(&bytes)?, "unchanged");
    assert_eq!(store.runtime_cursor(&profile)?.next_at, u64::MAX);
    store
        .connection
        .execute("DELETE FROM verification_consumer", [])?;
    cursor.pending = None;
    cursor.preparation = None;
    store.save_runtime_cursor(&profile, &cursor)?;
    assert_eq!(store.consume_verification(&bytes)?, "unchanged");
    Ok(())
}

#[test]
fn malformed_unrecognized_and_conflicting_facts_are_durably_rejected() -> TestResult {
    let mut store = CleanStore::in_memory()?;
    let (profile, bytes) = seed(&mut store)?;
    assert_eq!(store.consume_verification(b"not JSON")?, "rejected");
    assert_eq!(store.consume_verification(b"not JSON")?, "rejected");
    let (identity, mut decision) = decode(&bytes)?;
    decision.report_sha256 = format!("sha256:{}", "a".repeat(64));
    assert_eq!(
        store.consume_verification(&encode(&identity, &decision)?)?,
        "rejected"
    );
    assert_eq!(store.runtime_cursor(&profile)?.next_at, u64::MAX);
    assert_eq!(store.consume_verification(&bytes)?, "woken");
    assert_eq!(
        store.consume_verification(&encode(&identity, &decision)?)?,
        "rejected"
    );
    let mut other = identity;
    other.id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        store.consume_verification(&encode(&other, &decision)?)?,
        "rejected"
    );
    assert_eq!(count(&store, "verification_consumer")?, 4);
    Ok(())
}

#[test]
fn failed_receipt_commit_rolls_back_cleanup_wake() -> TestResult {
    let mut store = CleanStore::in_memory()?;
    let (profile, bytes) = seed(&mut store)?;
    store.connection.execute_batch("CREATE TRIGGER fail_delivery BEFORE INSERT ON verification_consumer BEGIN SELECT RAISE(ABORT,'fixture'); END;")?;
    assert!(store.consume_verification(&bytes).is_err());
    assert_eq!(store.runtime_cursor(&profile)?.next_at, u64::MAX);
    assert_eq!(count(&store, "verification_consumer")?, 0);
    Ok(())
}

#[test]
fn schema_fourteen_migration_preserves_outbox_and_bounded_retention() -> TestResult {
    let dir = tempdir()?;
    let path = dir.path().join("migration.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (_, bytes) = seed(&mut store)?;
    store
        .connection
        .execute_batch("DROP TABLE project_planner_sessions; DROP TABLE project_context_summaries; DROP TABLE project_context_records; DROP TABLE workspaces; DROP TABLE project_conversation_actions; DROP TABLE project_conversations; DROP TABLE project_agent_requests; DROP TABLE project_outcome_summaries; DROP TABLE lifecycle_outbox; DROP TABLE lifecycle_journal; DROP TABLE coding_inbox; DROP TABLE coding_executions; DROP TABLE runtime_operation_inbox; DROP TABLE verification_consumer; UPDATE clean_schema SET version=14;")?;
    drop(store);
    let mut store = CleanStore::open(path, 3)?;
    assert_eq!(store.schema_version()?, SCHEMA_VERSION);
    assert_eq!(store.consume_verification(&bytes)?, "woken");
    store
        .connection
        .execute("UPDATE verification_consumer SET processed_at=0", [])?;
    store.connection.execute_batch("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<150) INSERT INTO verification_consumer(delivery_key,digest,outcome,processed_at) SELECT printf('%064d',i),zeroblob(32),'rejected',0 FROM n;")?;
    store.prune_verification_deliveries()?;
    assert_eq!(count(&store, "verification_consumer")?, 23);
    assert_eq!(count(&store, "verification_outbox")?, 1);
    store.prune_verification_deliveries()?;
    assert_eq!(count(&store, "verification_consumer")?, 1);
    assert_eq!(store.consume_verification(&bytes)?, "duplicate");
    let (_, decision) = decode(&bytes)?;
    let mut cursor = store.runtime_cursor(&decision.runtime_profile_id)?;
    cursor.pending = None;
    cursor.preparation = None;
    store.save_runtime_cursor(&decision.runtime_profile_id, &cursor)?;
    store.prune_verification_deliveries()?;
    assert_eq!(count(&store, "verification_consumer")?, 0);
    Ok(())
}

#[test]
fn full_receipt_storage_defers_new_delivery_without_waking_or_losing_duplicates() -> TestResult {
    let mut store = CleanStore::in_memory()?;
    let (profile, bytes) = seed(&mut store)?;
    store.connection.execute_batch("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<100000) INSERT INTO verification_consumer(delivery_key,digest,outcome,processed_at) SELECT printf('%064d',i),zeroblob(32),'rejected',unixepoch() FROM n;")?;
    assert!(matches!(
        store.consume_verification(&bytes),
        Err(CleanStoreError::BoundExceeded)
    ));
    assert_eq!(store.runtime_cursor(&profile)?.next_at, u64::MAX);
    assert_eq!(count(&store, "verification_consumer")?, 100_000);
    store.connection.execute(
        "DELETE FROM verification_consumer WHERE delivery_key=printf('%064d',1)",
        [],
    )?;
    assert_eq!(store.consume_verification(&bytes)?, "woken");
    assert_eq!(store.consume_verification(&bytes)?, "duplicate");
    assert_eq!(count(&store, "verification_consumer")?, 100_000);
    Ok(())
}
