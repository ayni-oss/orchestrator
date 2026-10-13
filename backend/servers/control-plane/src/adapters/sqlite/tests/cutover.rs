//! Per-generation authority, transactional receipt acceptance and failure replay.
use super::verification::{accepted, count, prepared, receipt};
use super::*;
use crate::profiles::ayni::{
    preparation_state::{RuntimeCursor, VerificationAuthority},
    verification::Evidence,
};
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{
        VerificationExecutionFailed, VerificationFailureCode, VerificationResultAvailable,
        verification_result_available::Outcome,
    },
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn event_generation(
    store: &mut CleanStore,
) -> TestResult<(
    String,
    RuntimeCursor,
    ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport,
    ayni_orchestrator_application::runtime_activation::ActivateRuntime,
    EventIdentity,
    VerificationResultAvailable,
)> {
    let (profile, mut cursor, report, command) = prepared(store)?;
    cursor
        .preparation
        .as_mut()
        .ok_or("preparation")?
        .verification_authority = VerificationAuthority::Events {
        reporting: ayni_orchestrator_runtime::verification::ReportingConfiguration {
            source: "ayni://orchestrator/supervisors/test".into(),
            endpoint: "http://control-plane:8080/events/verification".into(),
        },
    };
    store.save_runtime_cursor(&profile, &cursor)?;
    let (identity, payload) = receipt(store, &profile, &cursor)?;
    Ok((profile, cursor, report, command, identity, payload))
}
fn decision(store: &CleanStore) -> TestResult<String> {
    Ok(store
        .connection
        .query_row("SELECT decision FROM verification_inbox", [], |r| r.get(0))?)
}

#[test]
fn event_acceptance_requires_receipt_and_commits_causation_atomically() -> TestResult {
    let directory = tempdir()?;
    let path = directory.path().join("events.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (profile, cursor, report, command, identity, payload) = event_generation(&mut store)?;
    let expected = serde_json::to_string(&cursor)?;
    let mut next = accepted(&cursor, &report)?;
    assert!(matches!(
        store.commit_verification(&command, &expected, &next),
        Err(CleanStoreError::InvalidTransition)
    ));
    store.receive_verification(&identity, &payload)?;
    store.decide_verification(&identity, &payload, &Evidence::Report(Box::new(report)))?;
    assert_eq!(decision(&store)?, "pending");
    assert_eq!(count(&store, "verification_journal")?, 0);
    next.pending.as_mut().ok_or("pending")?.observation = Some(identity.clone());
    store.connection.execute_batch("CREATE TRIGGER fail_outbox BEFORE INSERT ON verification_outbox BEGIN SELECT RAISE(ABORT,'failure'); END;")?;
    assert!(
        store
            .commit_verification(&command, &expected, &next)
            .is_err()
    );
    assert_eq!(decision(&store)?, "pending");
    assert_eq!(count(&store, "verification_journal")?, 0);
    store.connection.execute_batch("DROP TRIGGER fail_outbox")?;
    store.commit_verification(&command, &expected, &next)?;
    assert_eq!(decision(&store)?, "accepted");
    assert_eq!(
        store.runtime_profile(&profile)?.state,
        v1::RuntimeProfileState::Ready as i32
    );
    let outgoing = store.unpublished_verification_events()?;
    let (_, fact) = ayni_orchestrator_events::decisions::decode(&outgoing[0].1)?;
    assert_eq!(
        fact.observation_source.as_deref(),
        Some(identity.source.as_str())
    );
    assert_eq!(fact.observation_id.as_deref(), Some(identity.id.as_str()));
    let watermark = store.event_watermark()?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    store.receive_verification(&identity, &payload)?;
    store.commit_verification(&command, &expected, &next)?;
    assert_eq!(store.unpublished_verification_events()?, outgoing);
    assert_eq!(store.event_watermark()?, watermark);
    store.connection.execute(
        "UPDATE verification_inbox SET decision='rejected',reason='invalid'",
        [],
    )?;
    drop(store);
    assert!(matches!(
        CleanStore::open(&path, 3),
        Err(CleanStoreError::CorruptState)
    ));
    Ok(())
}

#[test]
fn event_failure_is_durable_once_and_rolls_back_with_receipt() -> TestResult {
    let directory = tempdir()?;
    let path = directory.path().join("failure.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (profile, _, _, _, identity, mut payload) = event_generation(&mut store)?;
    payload.outcome = Some(Outcome::ExecutionFailed(VerificationExecutionFailed {
        code: VerificationFailureCode::LockMismatch as i32,
        diagnostic: None,
    }));
    store.receive_verification(&identity, &payload)?;
    store.connection.execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE OF decision ON verification_inbox WHEN NEW.decision='accepted' BEGIN SELECT RAISE(ABORT,'failure'); END;")?;
    assert!(
        store
            .decide_verification(&identity, &payload, &Evidence::Failed)
            .is_err()
    );
    assert_eq!(store.runtime_cursor(&profile)?.failures, 0);
    assert_eq!(decision(&store)?, "pending");
    store
        .connection
        .execute_batch("DROP TRIGGER fail_receipt")?;
    store.decide_verification(&identity, &payload, &Evidence::Failed)?;
    assert_eq!(store.runtime_cursor(&profile)?.failures, 1);
    assert!(
        store
            .runtime_cursor(&profile)?
            .pending
            .as_ref()
            .is_some_and(|p| p.failed)
    );
    assert_eq!(decision(&store)?, "accepted");
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    store.receive_verification(&identity, &payload)?;
    store.decide_verification(&identity, &payload, &Evidence::Failed)?;
    assert_eq!(store.runtime_cursor(&profile)?.failures, 1);
    assert_eq!(count(&store, "verification_journal")?, 0);
    Ok(())
}

#[test]
fn event_receipts_cannot_resurrect_stale_or_expired_generations() -> TestResult {
    for race in ["generation", "cancellation", "deadline", "source"] {
        let mut store = CleanStore::in_memory()?;
        let (profile, mut cursor, _, _, identity, mut payload) = event_generation(&mut store)?;
        store.receive_verification(&identity, &payload)?;
        match race {
            "generation" => {
                cursor.preparation.as_mut().ok_or("preparation")?.generation = "replacement".into();
            }
            "cancellation" => cursor.pending.as_mut().ok_or("pending")?.failed = true,
            "deadline" => cursor.pending.as_mut().ok_or("pending")?.started_at = 0,
            _ => {
                payload.binding.as_mut().ok_or("binding")?.source_revision = "a".repeat(40);
                // Replace the test fixture with an authenticated but obsolete binding.
                store.connection.execute(
                    "UPDATE verification_inbox SET payload=?1",
                    [prost::Message::encode_to_vec(&payload)],
                )?;
            }
        }
        store.save_runtime_cursor(&profile, &cursor)?;
        store.decide_verification(&identity, &payload, &Evidence::Failed)?;
        assert_eq!(decision(&store)?, "rejected", "{race}");
        assert_eq!(store.runtime_cursor(&profile)?.failures, 0);
        assert_eq!(count(&store, "verification_journal")?, 0);
    }
    Ok(())
}

#[test]
fn reporting_failures_do_not_consume_physical_execution_budget() -> TestResult {
    for code in [
        VerificationFailureCode::RequestInvalid,
        VerificationFailureCode::ReportingFailed,
    ] {
        let mut store = CleanStore::in_memory()?;
        let (profile, _, _, _, identity, mut payload) = event_generation(&mut store)?;
        payload.outcome = Some(Outcome::ExecutionFailed(VerificationExecutionFailed {
            code: code as i32,
            diagnostic: None,
        }));
        store.receive_verification(&identity, &payload)?;
        store.decide_verification(&identity, &payload, &Evidence::Failed)?;
        assert_eq!(store.runtime_cursor(&profile)?.failures, 0);
        assert!(
            !store
                .runtime_cursor(&profile)?
                .pending
                .as_ref()
                .is_some_and(|p| p.failed)
        );
        assert_eq!(decision(&store)?, "accepted");
        assert_eq!(count(&store, "verification_journal")?, 0);
        store.decide_verification(&identity, &payload, &Evidence::Failed)?;
        assert_eq!(
            store
                .runtime_cursor(&profile)?
                .accounting
                .operational
                .values()
                .sum::<u32>(),
            1
        );
    }
    Ok(())
}

#[test]
fn schema_twelve_upgrade_preserves_receipts_and_legacy_authority() -> TestResult {
    let directory = tempdir()?;
    let path = directory.path().join("upgrade.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (profile, cursor, _, _) = prepared(&mut store)?;
    let (identity, payload) = receipt(&store, &profile, &cursor)?;
    store.receive_verification(&identity, &payload)?;
    store.connection.execute_batch("DROP TABLE lifecycle_outbox; DROP TABLE lifecycle_journal; DROP TABLE coding_inbox; DROP TABLE coding_executions; DROP TABLE runtime_operation_inbox; DROP TABLE verification_consumer; DROP INDEX verification_outbox_pending; DROP INDEX verification_outbox_stored; ALTER TABLE verification_outbox DROP COLUMN next_attempt_at; ALTER TABLE verification_outbox DROP COLUMN broker_sequence; ALTER TABLE verification_outbox DROP COLUMN last_error; UPDATE clean_schema SET version=12; UPDATE verification_inbox SET decision='shadow_accepted',reason='shadow_only'; UPDATE reconciliation_cursors SET value=json_remove(value,'$.preparation.verification_authority') WHERE name LIKE 'managed:%';")?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(store.schema_version()?, SCHEMA_VERSION);
    assert_eq!(decision(&store)?, "shadow_accepted");
    assert!(matches!(
        store
            .runtime_cursor(&profile)?
            .preparation
            .as_ref()
            .ok_or("preparation")?
            .verification_authority,
        VerificationAuthority::Retained
    ));
    store.receive_verification(&identity, &payload)?;
    assert_eq!(decision(&store)?, "shadow_accepted");
    Ok(())
}

/// Generates synthetic admitted state for the Docker Desktop test harness.
#[test]
#[ignore = "requires an explicit disposable fixture directory and pinned local worker"]
fn export_verification_cutover_fixture() -> TestResult {
    let directory = std::path::PathBuf::from(std::env::var("AYNI_VERIFICATION_FIXTURE_DIRECTORY")?);
    let image = std::env::var("AYNI_VERIFICATION_FIXTURE_WORKER")?;
    let digest = image.rsplit_once('@').ok_or("pinned worker")?.1;
    std::fs::create_dir_all(&directory)?;
    let mut store = CleanStore::open(directory.join("orchestrator.sqlite"), 3)?;
    let (profile, mut cursor, _, _, _, _) = event_generation(&mut store)?;
    // This fixture starts after onboarding; no real provider is configured in
    // the disposable chart deployment. Keep background onboarding from
    // invalidating the synthetic admitted generation.
    let repository =
        store.repository_connection(&store.runtime_profile(&profile)?.repository_connection_id)?;
    store.transition_repository_connection(
        &repository.id,
        repository.resource_version,
        v1::RepositoryConnectionStatus::Connected,
        None,
    )?;
    store.record_onboarding(&repository.id, "ready", "", SECOND_REVISION)?;
    let pending = cursor.pending.as_mut().ok_or("pending")?;
    pending.name = "verification-receipt-smoke".into();
    pending.resolved_digest = Some(digest.into());
    let reporting = ayni_orchestrator_runtime::verification::ReportingConfiguration {
        source: "ayni://orchestrator/supervisors/validation".into(),
        endpoint: "http://control-plane:8080/events/verification".into(),
    };
    cursor
        .preparation
        .as_mut()
        .ok_or("generation")?
        .verification_authority = VerificationAuthority::Events {
        reporting: reporting.clone(),
    };
    store.save_runtime_cursor(&profile, &cursor)?;
    if std::env::var("AYNI_RUNTIME_OPERATION_FIXTURE").as_deref() == Ok("true") {
        super::runtime_operations::seed_publication(&mut store, &profile, &mut cursor, &reporting)?;
    }
    let (_, mut payload) = receipt(&store, &profile, &cursor)?;
    let binding = payload.binding.as_mut().ok_or("binding")?;
    binding.resolved_image_digest = digest.into();
    binding.pod_uid.clear();
    let admission = ayni_orchestrator_runtime::verification::ReportingAdmission {
        configuration: reporting,
        binding: binding.clone(),
    };
    std::fs::write(
        directory.join("reporting.json"),
        serde_json::to_vec(&admission)?,
    )?;
    std::fs::write(
        directory.join("execution.json"),
        serde_json::to_vec(&cursor.pending.as_ref().ok_or("pending")?.request)?,
    )?;
    std::fs::write(directory.join("profile-id"), profile.as_bytes())?;
    assert_eq!(store.runtime_cursor(&profile)?.failures, 0);
    if std::env::var("AYNI_VERIFICATION_FIXTURE_TRACING").as_deref() == Ok("true") {
        fixture_trace(&store, &directory)?;
    }
    drop(store);
    let store = CleanStore::open(directory.join("orchestrator.sqlite"), 3)?;
    assert!(
        store
            .runtime_cursor(&profile)?
            .preparation
            .as_ref()
            .is_some_and(|p| p.verification_authority.events())
    );
    Ok(())
}

fn fixture_trace(store: &CleanStore, directory: &std::path::Path) -> TestResult {
    let guard = ayni_orchestrator_observability::init("verification-test-launcher");
    let span = ayni_orchestrator_observability::verification_span(
        "fixture_launch",
        "verification-receipt-smoke",
        None,
    );
    let context = span
        .in_scope(ayni_orchestrator_observability::current_context)
        .ok_or("fixture trace context")?;
    store.retain_verification_trace("verification-receipt-smoke", &context);
    std::fs::write(directory.join("traceparent"), context.traceparent)?;
    drop(span);
    guard.shutdown();
    Ok(())
}
