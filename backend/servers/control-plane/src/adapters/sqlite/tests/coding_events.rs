use super::*;
use ayni_orchestrator_events::{
    EventIdentity, coding, coding_decisions,
    v1::{CodingReportAvailable, CodingReportKind},
};
use ayni_orchestrator_runtime::verification::ReportingConfiguration;
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn configuration() -> ReportingConfiguration {
    ReportingConfiguration {
        source: "ayni://orchestrator/supervisors/test".into(),
        endpoint: "http://control-plane/events/verification".into(),
    }
}
fn admitted(
    store: &mut CleanStore,
) -> Result<
    (
        v1::DevelopmentTask,
        v1::DevelopmentAttempt,
        CodingReportAvailable,
    ),
    Box<dyn std::error::Error>,
> {
    let (repository, profile, _) = ready_profile(store)?;
    let (task, attempt) = store.create_development_task(
        "coding-task",
        &task_request(store, &repository.id, &profile.id, "coding")?,
    )?;
    let admission = store
        .select_coding_execution(&attempt.id, "coding-one", Some(configuration()))?
        .ok_or("admission")?;
    store.bind_coding_request(&attempt.id, &admission.binding.generation, DIGEST)?;
    let mut binding = admission.binding;
    binding.pod_uid = "pod-one".into();
    binding.request_digest = DIGEST.into();
    let report = CodingReportAvailable {
        binding: Some(binding),
        kind: CodingReportKind::Final as i32,
        sequence: 257,
        sha256: DIGEST.into(),
        size_bytes: 100,
        contract: coding::CONTRACT.into(),
    };
    Ok((task, store.development_attempt(&attempt.id)?, report))
}
fn identity() -> EventIdentity {
    EventIdentity {
        source: configuration().source,
        id: Uuid::now_v7().to_string(),
    }
}
fn usage() -> v1::AttemptModelUsage {
    v1::AttemptModelUsage {
        collection_status: v1::ModelUsageCollectionStatus::Unavailable as i32,
        ..Default::default()
    }
}
fn result() -> v1::WorkerResult {
    v1::WorkerResult {
        execution: v1::ExecutionOutcome::NoChanges as i32,
        quality: Some(v1::QualityResult {
            outcome: v1::QualityOutcome::Unavailable as i32,
            ..Default::default()
        }),
        publication: v1::PublicationOutcome::NotRequired as i32,
        model_usage: Some(usage()),
        ..Default::default()
    }
}
fn accept(
    store: &mut CleanStore,
    id: &EventIdentity,
    report: &CodingReportAvailable,
) -> Result<bool, CleanStoreError> {
    store.commit_coding_report(id, report, None, Some(result()), vec![], usage())
}
#[test]
fn final_receipt_commits_once_before_exit_and_survives_restart() -> TestResult {
    let dir = tempdir()?;
    let path = dir.path().join("coding.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (_, attempt, report) = admitted(&mut store)?;
    let id = identity();
    store.receive_coding(&id, &report)?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    assert!(accept(&mut store, &id, &report)?);
    assert!(!accept(&mut store, &id, &report)?);
    store.receive_coding(&id, &report)?;
    assert_eq!(
        store.development_attempt(&attempt.id)?.state,
        v1::DevelopmentAttemptState::Succeeded as i32
    );
    let facts = store.claim_coding_publications(unix_seconds()? + 600, 8)?;
    assert_eq!(facts.len(), 1);
    let (_, fact) = coding_decisions::decode(&facts[0].bytes)?;
    assert!(fact.applied);
    assert_eq!(fact.observation, Some(report.clone()));
    store.record_coding_publication(&facts[0], None)?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    let replay = store.claim_coding_publications(unix_seconds()? + 1200, 8)?;
    assert_eq!(replay, facts);
    store.record_coding_publication(&facts[0], Some(1))?;
    assert!(
        store
            .claim_coding_publications(unix_seconds()? + 1800, 8)?
            .is_empty()
    );
    let mut conflict = report;
    conflict.sha256 = SECOND_DIGEST.into();
    assert!(matches!(
        store.receive_coding(&id, &conflict),
        Err(CleanStoreError::IdempotencyConflict)
    ));
    Ok(())
}
#[test]
fn final_acceptance_rolls_back_projection_when_journal_write_fails() -> TestResult {
    let mut store = CleanStore::in_memory()?;
    let (_, attempt, report) = admitted(&mut store)?;
    let id = identity();
    store.receive_coding(&id, &report)?;
    store.connection.execute_batch("CREATE TRIGGER block_coding_fact BEFORE UPDATE OF accepted_payload ON coding_inbox BEGIN SELECT RAISE(ABORT,'injected'); END;")?;
    assert!(accept(&mut store, &id, &report).is_err());
    assert_eq!(store.development_attempt(&attempt.id)?, attempt);
    assert_eq!(store.pending_coding(&attempt.id)?.len(), 1);
    store
        .connection
        .execute_batch("DROP TRIGGER block_coding_fact;")?;
    assert!(accept(&mut store, &id, &report)?);
    Ok(())
}
#[test]
fn cancelled_and_expired_tasks_retain_late_facts_without_resurrection() -> TestResult {
    for expired in [false, true] {
        let mut store = CleanStore::in_memory()?;
        let (task, attempt, report) = admitted(&mut store)?;
        if expired {
            let mut value = attempt.clone();
            value.snapshot.as_mut().ok_or("snapshot")?.deadline_at = Some(timestamp(1));
            store.connection.execute(
                "UPDATE development_attempts SET payload=?2 WHERE id=?1",
                params![attempt.id, value.encode_to_vec()],
            )?;
        } else {
            store.cancel_development_task(
                "cancel",
                &v1::CancelDevelopmentTaskRequest {
                    idempotency_key: "cancel".into(),
                    development_task_id: task.id,
                },
            )?;
        }
        let before = store.development_attempt(&attempt.id)?;
        let id = identity();
        store.receive_coding(&id, &report)?;
        assert!(!accept(&mut store, &id, &report)?);
        assert_eq!(store.development_attempt(&attempt.id)?, before);
        let facts = store.claim_coding_publications(unix_seconds()? + 600, 8)?;
        assert!(!coding_decisions::decode(&facts[0].bytes)?.1.applied);
    }
    Ok(())
}
#[test]
fn authority_rotates_only_after_stop_and_old_generations_cannot_complete_replacement() -> TestResult
{
    let mut store = CleanStore::in_memory()?;
    let (_, attempt, old) = admitted(&mut store)?;
    let first = store.coding_admission(&attempt.id)?.ok_or("admission")?;
    assert_eq!(
        store
            .select_coding_execution(&attempt.id, "coding-one", None)?
            .ok_or("admission")?
            .binding,
        first.binding
    );
    store.stop_coding_execution(&attempt.id)?;
    let replacement = store
        .select_coding_execution(&attempt.id, "coding-one", None)?
        .ok_or("admission")?;
    assert_ne!(first.binding.generation, replacement.binding.generation);
    let before = store.development_attempt(&attempt.id)?;
    let id = identity();
    store.receive_coding(&id, &old)?;
    assert!(!accept(&mut store, &id, &old)?);
    assert_eq!(store.development_attempt(&attempt.id)?, before);
    Ok(())
}
#[test]
fn reordered_advisory_reports_do_not_regress_budgets_or_block_final() -> TestResult {
    let mut store = CleanStore::in_memory()?;
    let (_, attempt, mut report) = admitted(&mut store)?;
    report.kind = CodingReportKind::Progress as i32;
    report.sequence = 2;
    let newer = identity();
    store.receive_coding(&newer, &report)?;
    let progress = v1::WorkerProgress {
        phase: v1::WorkerPhase::Coding as i32,
        coding_turns_consumed: 2,
        ..Default::default()
    };
    assert!(store.commit_coding_report(
        &newer,
        &report,
        Some(progress.clone()),
        None,
        vec![],
        usage()
    )?);
    report.sequence = 1;
    let older = identity();
    store.receive_coding(&older, &report)?;
    assert!(!store.commit_coding_report(
        &older,
        &report,
        Some(v1::WorkerProgress::default()),
        None,
        vec![],
        usage()
    )?);
    assert_eq!(
        store.development_attempt(&attempt.id)?.progress,
        Some(progress)
    );
    report.sequence = 257;
    report.kind = CodingReportKind::Final as i32;
    let final_id = identity();
    store.receive_coding(&final_id, &report)?;
    let mut final_result = result();
    final_result.coding_turns_consumed = 2;
    assert!(store.commit_coding_report(
        &final_id,
        &report,
        None,
        Some(final_result),
        vec![],
        usage()
    )?);
    Ok(())
}
#[test]
fn schema_sixteen_migration_freezes_existing_attempts_on_legacy_reader() -> TestResult {
    let dir = tempdir()?;
    let path = dir.path().join("legacy.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (_, attempt, _) = admitted(&mut store)?;
    store.connection.execute_batch(
        "DROP TABLE project_conversation_actions; DROP TABLE project_conversations; DROP TABLE project_agent_requests; DROP TABLE project_outcome_summaries; DROP TABLE lifecycle_outbox; DROP TABLE lifecycle_journal; DROP TABLE coding_inbox;DROP TABLE coding_executions;UPDATE clean_schema SET version=16;",
    )?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(store.schema_version()?, SCHEMA_VERSION);
    assert!(
        store
            .select_coding_execution(&attempt.id, "coding-one", Some(configuration()))?
            .is_none()
    );
    store.stop_coding_execution(&attempt.id)?;
    assert!(
        store
            .select_coding_execution(&attempt.id, "coding-one", Some(configuration()))?
            .is_none()
    );
    Ok(())
}

#[test]
#[ignore = "exports admitted state for the isolated Docker Desktop coding fixture"]
fn export_coding_cutover_fixture() -> TestResult {
    let directory = std::path::PathBuf::from(std::env::var("AYNI_CODING_FIXTURE_DIRECTORY")?);
    std::fs::create_dir_all(&directory)?;
    let mut store = CleanStore::open(directory.join("orchestrator.sqlite"), 3)?;
    let (task, attempt, _) = admitted(&mut store)?;
    let repository = store.repository_connection(
        &task
            .snapshot
            .as_ref()
            .and_then(|s| s.repository.as_ref())
            .ok_or("source")?
            .repository_connection_id,
    )?;
    store.transition_repository_connection(
        &repository.id,
        repository.resource_version,
        v1::RepositoryConnectionStatus::Connected,
        None,
    )?;
    store.record_onboarding(&repository.id, "ready", "", REVISION)?;
    let mut admission = store.coding_admission(&attempt.id)?.ok_or("admission")?;
    admission.binding.execution_id =
        ayni_orchestrator_kubernetes::execution::operation_name(&attempt.id, 0);
    admission.configuration.source = "ayni://orchestrator/supervisors/validation".into();
    admission.configuration.endpoint = "http://control-plane:8080/events/verification".into();
    store.connection.execute(
        "UPDATE coding_executions SET admission=?2,request_digest='' WHERE attempt_id=?1",
        params![attempt.id, serde_json::to_string(&admission)?],
    )?;
    let spec: ayni_orchestrator_profile_ayni::coding::WorkerSpec = serde_json::from_value(
        serde_json::json!({
            "task_id":task.id,"attempt_id":attempt.id,
            "execution":{"repository_url":"https://github.com/ayni-oss/sandbox.git","revision":REVISION,"expected":{"lock_fingerprint":DIGEST,"platform":"linux/arm64","ayni_version":"0.15.0"}},
            "runtime_digest":DIGEST,"request":"Validate authenticated admission failure","acceptance_criteria":["Truthful failure survives restart"],"constraints":{"protected_paths":[],"allow_test_edits":false},"model":"fixture",
            "limits":{"coding_turns":24,"quality_repair_limit":3,"deadline_at":unix_seconds()?+3600},
            "publication":{"repository":"ayni-oss/sandbox","base_branch":"main","branch":format!("task/{}",attempt.id.replace(':',"-"))},
            "publication_only":false,"reporting":admission
        }),
    )?;
    let bytes = serde_json::to_vec(&spec)?;
    let digest = ayni_orchestrator_domain::digest::sha256(&bytes);
    store.bind_coding_request(&attempt.id, &admission.binding.generation, &digest)?;
    admission.binding.request_digest = digest;
    std::fs::write(directory.join("worker.json"), bytes)?;
    std::fs::write(
        directory.join("coding-reporting.json"),
        serde_json::to_vec(&admission)?,
    )?;
    std::fs::write(directory.join("attempt-id"), attempt.id.as_bytes())?;
    std::fs::write(
        directory.join("execution-name"),
        admission.binding.execution_id.as_bytes(),
    )?;
    store.transition_attempt(
        &attempt.id,
        attempt.resource_version,
        v1::DevelopmentAttemptState::Running,
        None,
    )?;
    Ok(())
}

#[test]
fn reporting_and_cleanup_projections_survive_restart_and_notify_watches() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("status.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (_, attempt, report) = admitted(&mut store)?;
    assert_eq!(attempt.reporting_state, "awaiting");
    assert!(attempt.cleanup_state.is_empty());
    let identity = identity();
    store.receive_coding(&identity, &report)?;
    assert!(accept(&mut store, &identity, &report)?);
    let accepted = store.development_attempt(&attempt.id)?;
    assert_eq!(accepted.reporting_state, "accepted");
    assert_eq!(accepted.cleanup_state, "pending");
    let cursor = store.current_cursor(DEVELOPMENT_SERVICE)?;
    store.coding_cleanup_complete(&attempt.id)?;
    let cleaned = store.development_attempt(&attempt.id)?;
    assert_eq!(cleaned.reporting_state, "accepted");
    assert_eq!(cleaned.cleanup_state, "complete");
    assert!(cleaned.resource_version > accepted.resource_version);
    assert!(store.current_cursor(DEVELOPMENT_SERVICE)? > cursor);
    let cleaned_cursor = store.current_cursor(DEVELOPMENT_SERVICE)?;
    store.coding_cleanup_complete(&attempt.id)?;
    assert_eq!(store.current_cursor(DEVELOPMENT_SERVICE)?, cleaned_cursor);
    drop(store);
    let store = CleanStore::open(&path, 3)?;
    assert_eq!(store.development_attempt(&attempt.id)?, cleaned);
    Ok(())
}
