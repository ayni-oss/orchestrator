//! Transaction, restart and late-observation invariants against real `SQLite`.
use super::*;
use crate::profiles::ayni::{preparation_state::RuntimeCursor, verification::Evidence};
use ayni_orchestrator_application::runtime_activation::ActivateRuntime;
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{
        VerificationEvidenceReference, VerificationExecutionBinding, VerificationReportAvailable,
        VerificationResultAvailable, verification_result_available::Outcome,
    },
};
use ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport;

fn prepared(
    store: &mut CleanStore,
) -> Result<(String, RuntimeCursor, VerificationReport, ActivateRuntime), Box<dyn std::error::Error>>
{
    let (_, profile, _) = ready_profile(store)?;
    let revision = runtime::select(
        store,
        &profile.id,
        SECOND_REVISION,
        &"c".repeat(64),
        "registry.test/team/runtime",
        DIGEST,
    )?;
    let report = runtime::report(SECOND_REVISION);
    let fields = serde_json::json!({
        "preparation": {"generation":"generation-one","source_revision":SECOND_REVISION,"policy_digest":DIGEST,"lock_digest":DIGEST,
          "contract_version":"0.15.0","platform":"linux/amd64","registry":[],"certificate":null,
          "runtime":{"image":"registry.test/team/runtime:stable","platform":"linux/amd64","service_account":"runtime-coding"}},
        "pending": {"name":"verify-one","profile_revision":revision,"request":{"repository_url":"https://github.com/ayni-oss/sandbox.git","revision":SECOND_REVISION,
          "expected":{"lock_fingerprint":format!("sha256:{}","c".repeat(64)),"platform":"linux/amd64","ayni_version":"0.15.0"}},
          "image":"registry.test/team/runtime:stable","started_at":unix_seconds()?,"source_digest":DIGEST,"lock_digest":DIGEST,"build_name":null,"phase":"verifying","resolved_digest":DIGEST,
          "certificate":{"id":"certificate","revision":"1","key_id":"test-producer","public_key":"public-fixture","secret_name":"fixture-reference","enabled":true,"status":"ready"}}
    });
    let mut wire = serde_json::to_value(RuntimeCursor::default())?;
    for (key, value) in fields.as_object().ok_or("fields")? {
        wire[key] = value.clone();
    }
    let cursor: RuntimeCursor = serde_json::from_value(wire)?;
    store.save_runtime_cursor(&profile.id, &cursor)?;
    let raw: String = store.connection.query_row(
        "SELECT value FROM reconciliation_cursors WHERE name=?1",
        [format!("managed:{}", profile.id)],
        |r| r.get(0),
    )?;
    let _: RuntimeCursor = serde_json::from_str(&raw)?;
    let command = crate::profiles::ayni::runtime_activation::command(
        &profile.id,
        &revision,
        "registry.test/team/runtime:stable",
        DIGEST,
        &report,
    )?;
    Ok((profile.id, cursor, report, command))
}

fn accepted(
    cursor: &RuntimeCursor,
    report: &VerificationReport,
) -> Result<RuntimeCursor, Box<dyn std::error::Error>> {
    let mut next = cursor.clone();
    let pending = next.pending.as_mut().ok_or("fixture pending")?;
    pending.verification = Some(report.clone());
    pending.phase =
        ayni_orchestrator_application::runtime_preparation::ImagePreparationPhase::Activating;
    next.status.insert("phase".into(), "cleanup".into());
    Ok(next)
}

fn count(store: &CleanStore, table: &str) -> Result<i64, CleanStoreError> {
    store
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .map_err(CleanStoreError::Database)
}

#[test]
fn acceptance_is_atomic_restartable_and_replay_does_not_duplicate_effects()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("accept.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (profile, cursor, report, command) = prepared(&mut store)?;
    assert_eq!(
        report.quality.outcome,
        ayni_orchestrator_runtime::coding::QualityOutcome::Failed
    );
    let expected = serde_json::to_string(&cursor)?;
    let next = accepted(&cursor, &report)?;
    store
        .commit_verification(&command, &expected, &next)
        .map_err(|e| std::io::Error::other(format!("commit {e:?}")))?;
    let watermark = store.event_watermark()?;
    let unpublished = store.unpublished_verification_events()?;
    assert_eq!(unpublished.len(), 1);
    assert_eq!(
        store.runtime_profile(&profile)?.state,
        v1::RuntimeProfileState::Ready as i32
    );
    assert!(
        store
            .runtime_cursor(&profile)?
            .pending
            .as_ref()
            .is_some_and(|p| p.verification.is_some())
    );
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    store.commit_verification(&command, &expected, &next)?;
    assert_eq!(store.event_watermark()?, watermark);
    assert_eq!(store.unpublished_verification_events()?, unpublished);
    assert_eq!(count(&store, "verification_journal")?, 1);
    assert_eq!(count(&store, "verification_outbox")?, 1);
    let bytes: Vec<u8> =
        store
            .connection
            .query_row("SELECT payload FROM verification_outbox", [], |r| r.get(0))?;
    let (_, decision) = ayni_orchestrator_events::decisions::decode(&bytes)?;
    assert_eq!(decision.runtime_profile_id, profile);
    assert_eq!(
        decision.aggregate_revision,
        store.runtime_profile(&profile)?.resource_version
    );
    assert_eq!(store.runtime_cursor(&profile)?.failures, 0);
    Ok(())
}

#[test]
fn conflicting_replay_and_corrupt_outbox_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("outbox.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (_, cursor, mut report, command) = prepared(&mut store)?;
    let expected = serde_json::to_string(&cursor)?;
    store.commit_verification(&command, &expected, &accepted(&cursor, &report)?)?;
    report.quality.outcome = ayni_orchestrator_runtime::coding::QualityOutcome::Passed;
    assert!(matches!(
        store.commit_verification(&command, &expected, &accepted(&cursor, &report)?),
        Err(CleanStoreError::IdempotencyConflict)
    ));
    assert_eq!(count(&store, "verification_journal")?, 1);
    assert_eq!(count(&store, "verification_outbox")?, 1);
    store
        .connection
        .execute("UPDATE verification_outbox SET payload=x'00'", [])?;
    drop(store);
    assert!(matches!(
        CleanStore::open(&path, 3),
        Err(CleanStoreError::CorruptState)
    ));
    Ok(())
}

#[test]
fn failure_during_outbox_write_rolls_back_projection_cursor_journal_and_client_events()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = CleanStore::in_memory()?;
    let (profile, cursor, report, command) = prepared(&mut store)?;
    let before = store.runtime_profile(&profile)?;
    let watermark = store.event_watermark()?;
    let artifacts = count(&store, "runtime_artifacts")?;
    store.connection.execute_batch("CREATE TRIGGER fail_outbox BEFORE INSERT ON verification_outbox BEGIN SELECT RAISE(ABORT,'fixture failure'); END;")?;
    assert!(
        store
            .commit_verification(
                &command,
                &serde_json::to_string(&cursor)?,
                &accepted(&cursor, &report)?
            )
            .is_err()
    );
    assert_eq!(store.runtime_profile(&profile)?, before);
    assert_eq!(store.event_watermark()?, watermark);
    assert_eq!(count(&store, "runtime_artifacts")?, artifacts);
    assert_eq!(count(&store, "verification_journal")?, 0);
    assert_eq!(
        serde_json::to_string(&store.runtime_cursor(&profile)?)?,
        serde_json::to_string(&cursor)?
    );
    store.connection.execute_batch("DROP TRIGGER fail_outbox")?;
    store.commit_verification(
        &command,
        &serde_json::to_string(&cursor)?,
        &accepted(&cursor, &report)?,
    )?;
    Ok(())
}

#[test]
fn selection_generation_cancellation_and_deadline_races_cannot_commit()
-> Result<(), Box<dyn std::error::Error>> {
    for race in ["selection", "generation", "cancellation", "deadline"] {
        let mut store = CleanStore::in_memory()?;
        let (profile, mut cursor, report, command) = prepared(&mut store)?;
        if race == "deadline" {
            cursor.pending.as_mut().ok_or("pending")?.started_at = 0;
            store.save_runtime_cursor(&profile, &cursor)?;
        }
        let expected = serde_json::to_string(&cursor)?;
        let next = accepted(&cursor, &report)?;
        match race {
            "selection" => {
                runtime::select(
                    &mut store,
                    &profile,
                    REVISION,
                    &"c".repeat(64),
                    "registry.test/team/runtime",
                    DIGEST,
                )?;
            }
            "generation" => {
                cursor.preparation.as_mut().ok_or("generation")?.generation =
                    "new-generation".into();
                store.save_runtime_cursor(&profile, &cursor)?;
            }
            "cancellation" => {
                cursor.pending.as_mut().ok_or("pending")?.failed = true;
                store.save_runtime_cursor(&profile, &cursor)?;
            }
            _ => (),
        }
        let watermark = store.event_watermark()?;
        assert!(
            store
                .commit_verification(&command, &expected, &next)
                .is_err(),
            "{race}"
        );
        assert_eq!(store.event_watermark()?, watermark, "{race}");
        assert_eq!(count(&store, "verification_journal")?, 0, "{race}");
    }
    Ok(())
}

fn receipt(
    store: &CleanStore,
    profile: &str,
    cursor: &RuntimeCursor,
) -> Result<(EventIdentity, VerificationResultAvailable), Box<dyn std::error::Error>> {
    let p = cursor.pending.as_ref().ok_or("pending")?;
    let g = cursor.preparation.as_ref().ok_or("generation")?;
    let b = VerificationExecutionBinding {
        repository_connection_id: store.runtime_profile(profile)?.repository_connection_id,
        runtime_profile_id: profile.into(),
        runtime_profile_revision_id: p.profile_revision.clone(),
        preparation_id: g.generation.clone(),
        execution_id: p.name.clone(),
        pod_uid: "pod-one".into(),
        request_digest: ayni_orchestrator_domain::digest::sha256(&serde_json::to_vec(&p.request)?),
        source_revision: p.request.revision.clone(),
        lock_digest: p.lock_digest.clone(),
        policy_digest: g.policy_digest.clone(),
        platform: g.platform.clone(),
        resolved_image_digest: DIGEST.into(),
    };
    Ok((
        EventIdentity {
            source: "ayni://orchestrator/supervisors/test".into(),
            id: Uuid::now_v7().to_string(),
        },
        VerificationResultAvailable {
            binding: Some(b),
            outcome: Some(Outcome::ReportAvailable(VerificationReportAvailable {
                evidence: Some(VerificationEvidenceReference {
                    evidence_id: "verification-json".into(),
                    sha256: DIGEST.into(),
                    size_bytes: 12,
                    contract: "retained-v1".into(),
                }),
            })),
        },
    ))
}

#[test]
fn shadows_are_durable_but_never_activate_and_unavailable_evidence_is_deferred()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = CleanStore::in_memory()?;
    let (profile, cursor, report, _) = prepared(&mut store)?;
    let (identity, payload) = receipt(&store, &profile, &cursor)?;
    ayni_orchestrator_events::encode(&identity, &payload)?;
    store.receive_verification(&identity, &payload)?;
    let before = store.runtime_profile(&profile)?;
    let watermark = store.event_watermark()?;
    store
        .decide_shadow_verification(&identity, &payload, &Evidence::Unavailable)
        .map_err(|e| std::io::Error::other(format!("shadow unavailable: {e:?}")))?;
    assert!(store.pending_verifications()?.is_empty());
    store.decide_shadow_verification(&identity, &payload, &Evidence::Report(Box::new(report)))?;
    store.receive_verification(&identity, &payload)?;
    assert_eq!(store.runtime_profile(&profile)?, before);
    assert_eq!(store.event_watermark()?, watermark);
    assert_eq!(count(&store, "verification_outbox")?, 0);
    let decision: String =
        store
            .connection
            .query_row("SELECT decision FROM verification_inbox", [], |r| r.get(0))?;
    assert_eq!(decision, "shadow_accepted");
    Ok(())
}

#[test]
fn shadow_rejection_survives_restart_and_delivery_replay() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempdir()?;
    let path = dir.path().join("shadow.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (profile, mut cursor, report, _) = prepared(&mut store)?;
    let (identity, payload) = receipt(&store, &profile, &cursor)?;
    store.receive_verification(&identity, &payload)?;
    cursor.pending.as_mut().ok_or("pending")?.failed = true;
    store.save_runtime_cursor(&profile, &cursor)?;
    let watermark = store.event_watermark()?;
    store.decide_shadow_verification(&identity, &payload, &Evidence::Report(Box::new(report)))?;
    drop(store);
    let mut store = CleanStore::open(path, 3)?;
    store.receive_verification(&identity, &payload)?;
    assert!(store.pending_verifications()?.is_empty());
    let (decision, reason): (String, String) =
        store
            .connection
            .query_row("SELECT decision,reason FROM verification_inbox", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
    assert_eq!(decision, "rejected");
    assert_eq!(reason, "verification_authority_stale");
    assert_eq!(store.event_watermark()?, watermark);
    assert_eq!(store.runtime_cursor(&profile)?.failures, 0);
    Ok(())
}

#[test]
fn cleanup_completion_cannot_overwrite_a_new_generation() -> Result<(), Box<dyn std::error::Error>>
{
    let mut store = CleanStore::in_memory()?;
    let (profile, cursor, report, command) = prepared(&mut store)?;
    let mut next = accepted(&cursor, &report)?;
    store.commit_verification(&command, &serde_json::to_string(&cursor)?, &next)?;
    let expected = serde_json::to_string(&store.runtime_cursor(&profile)?)?;
    let mut changed = next.clone();
    changed.preparation.as_mut().ok_or("generation")?.generation = "replacement".into();
    store.save_runtime_cursor(&profile, &changed)?;
    next.pending = None;
    next.preparation = None;
    assert!(matches!(
        store.complete_verification_cleanup(&profile, &expected, &next),
        Err(CleanStoreError::StaleVersion)
    ));
    assert_eq!(
        store
            .runtime_cursor(&profile)?
            .preparation
            .ok_or("generation")?
            .generation,
        "replacement"
    );
    Ok(())
}

#[test]
fn bounded_evidence_resolver_checks_hash_physical_binding_and_special_files()
-> Result<(), Box<dyn std::error::Error>> {
    use ayni_orchestrator_runtime::verification::{VerificationOutcome, VerificationRecord};
    let mut store = CleanStore::in_memory()?;
    let (profile, cursor, report, _) = prepared(&mut store)?;
    let (_, mut payload) = receipt(&store, &profile, &cursor)?;
    let record = VerificationRecord {
        version: "retained-v1".into(),
        execution: "verify-one".into(),
        pod_uid: "pod-one".into(),
        request_digest: payload
            .binding
            .as_ref()
            .ok_or("binding")?
            .request_digest
            .clone(),
        outcome: VerificationOutcome::Report { report },
    };
    let bytes = serde_json::to_vec(&record)?;
    let Some(Outcome::ReportAvailable(ref mut available)) = payload.outcome else {
        return Err("report".into());
    };
    let evidence = available.evidence.as_mut().ok_or("reference")?;
    evidence.sha256 = ayni_orchestrator_domain::digest::sha256(&bytes);
    evidence.size_bytes = u64::try_from(bytes.len())?;
    let directory = tempdir()?;
    std::fs::create_dir(directory.path().join("verify-one"))?;
    let path = directory.path().join("verify-one/verification.json");
    std::fs::write(&path, &bytes)?;
    assert!(matches!(
        crate::profiles::ayni::verification::read(directory.path(), &payload),
        Evidence::Report(_)
    ));
    payload.binding.as_mut().ok_or("binding")?.pod_uid = "other-pod".into();
    assert!(matches!(
        crate::profiles::ayni::verification::read(directory.path(), &payload),
        Evidence::Invalid("evidence_binding_mismatch")
    ));
    std::fs::write(&path, b"changed")?;
    assert!(matches!(
        crate::profiles::ayni::verification::read(directory.path(), &payload),
        Evidence::Invalid("evidence_digest_mismatch")
    ));
    std::fs::remove_file(&path)?;
    std::fs::create_dir(&path)?;
    assert!(matches!(
        crate::profiles::ayni::verification::read(directory.path(), &payload),
        Evidence::Invalid("evidence_unsafe_or_oversized")
    ));
    Ok(())
}

#[test]
fn schema_eleven_migration_preserves_pending_receipts() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let path = dir.path().join("migrate.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (profile, cursor, _, _) = prepared(&mut store)?;
    let (identity, payload) = receipt(&store, &profile, &cursor)?;
    store.receive_verification(&identity, &payload)?;
    store.connection.execute_batch("DROP TABLE verification_outbox; DROP TABLE verification_journal;
ALTER TABLE verification_inbox RENAME TO inbox_new; DROP INDEX verification_inbox_pending;
CREATE TABLE verification_inbox(source TEXT NOT NULL,event_id TEXT NOT NULL,execution_id TEXT NOT NULL,pod_uid TEXT NOT NULL,payload BLOB NOT NULL,received_at INTEGER NOT NULL,decision TEXT NOT NULL CHECK(decision='pending'),PRIMARY KEY(source,event_id),UNIQUE(execution_id,pod_uid));
INSERT INTO verification_inbox SELECT source,event_id,execution_id,pod_uid,payload,received_at,decision FROM inbox_new;
DROP TABLE inbox_new; CREATE INDEX verification_inbox_pending ON verification_inbox(received_at,source,event_id);
UPDATE clean_schema SET version=11 WHERE component='control_plane';")?;
    drop(store);
    let mut store = CleanStore::open(path, 3)?;
    assert_eq!(store.schema_version()?, 12);
    assert_eq!(
        store.pending_verifications()?,
        vec![(identity.clone(), payload.clone())]
    );
    store.receive_verification(&identity, &payload)?;
    assert_eq!(count(&store, "verification_inbox")?, 1);
    Ok(())
}

#[tokio::test]
async fn startup_drain_defers_missing_evidence_without_starving_stale_receipts()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let path = dir.path().join("drain.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (profile, cursor, _, _) = prepared(&mut store)?;
    let (identity, payload) = receipt(&store, &profile, &cursor)?;
    store.receive_verification(&identity, &payload)?;
    let mut previous = payload.clone();
    previous.binding.as_mut().ok_or("binding")?.execution_id = "old-execution".into();
    store.receive_verification(
        &EventIdentity {
            source: identity.source.clone(),
            id: Uuid::now_v7().to_string(),
        },
        &previous,
    )?;
    drop(store);
    let state = crate::AppState::open(&path, 3)?;
    state
        .verification_observations_at(dir.path().to_owned())
        .await?;
    state.shutdown_database().await?;
    let store = CleanStore::open(&path, 3)?;
    let deferred: i64 = store.connection.query_row(
        "SELECT COUNT(*) FROM verification_inbox WHERE decision='pending' AND next_attempt_at>0",
        [],
        |r| r.get(0),
    )?;
    let rejected: i64 = store.connection.query_row(
        "SELECT COUNT(*) FROM verification_inbox WHERE decision='rejected'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!((deferred, rejected), (1, 1));
    assert_eq!(count(&store, "verification_journal")?, 0);
    Ok(())
}
