use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn approved()
-> Result<(CleanStore, v1::ProjectConversation, v1::DevelopmentAttempt), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("evidence-message", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    let c = store.decide_project_plan("evidence-approval", &plan_decision(&c, "approve")?)?;
    let attempt = store.development_attempt(&c.proposals[0].attempt_id)?;
    Ok((store, c, attempt))
}
fn worker_result(
    attempt: &v1::DevelopmentAttempt,
    execution: v1::ExecutionOutcome,
    quality: v1::QualityOutcome,
    publication: v1::PublicationOutcome,
) -> v1::WorkerResult {
    let commit = REVISION.to_string();
    v1::WorkerResult {
        execution: execution as i32,
        commit: Some(commit.clone()),
        changed_paths: vec!["src/lib.rs".into()],
        quality: Some(v1::QualityResult {
            outcome: quality as i32,
            evidence: (quality != v1::QualityOutcome::Unavailable).then(|| v1::QualityEvidence {
                artifact_path: "evidence/signals.json".into(),
                signals_digest: DIGEST.into(),
                source_fingerprint: SECOND_DIGEST.into(),
                commit: commit.clone(),
            }),
            findings: if quality == v1::QualityOutcome::Failed {
                vec!["Required test failed".into()]
            } else {
                vec![]
            },
            ..Default::default()
        }),
        publication: publication as i32,
        pull_request: (publication == v1::PublicationOutcome::Published).then(|| {
            v1::PullRequestReceipt {
                repository: "ayni-oss/sandbox".into(),
                number: 42,
                url: "https://github.com/ayni-oss/sandbox/pull/42".into(),
                branch: format!("task/{}", attempt.id.replace(':', "-")),
                base_branch: "main".into(),
                head_commit: commit,
                draft: true,
            }
        }),
        requested_input: (execution == v1::ExecutionOutcome::InputRequired)
            .then(|| "Which compatibility behavior should be preserved?".into()),
        model_usage: Some(v1::AttemptModelUsage {
            collection_status: v1::ModelUsageCollectionStatus::Unavailable as i32,
            ..Default::default()
        }),
        ..Default::default()
    }
}
#[test]
fn worker_outcomes_preserve_independent_evidence_and_do_not_start_agents() -> TestResult {
    for (execution, quality, publication) in [
        (
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Passed,
            v1::PublicationOutcome::Published,
        ),
        (
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Failed,
            v1::PublicationOutcome::Published,
        ),
        (
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Unavailable,
            v1::PublicationOutcome::Published,
        ),
        (
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Passed,
            v1::PublicationOutcome::Failed,
        ),
        (
            v1::ExecutionOutcome::Failed,
            v1::QualityOutcome::Unavailable,
            v1::PublicationOutcome::NotRequired,
        ),
        (
            v1::ExecutionOutcome::InputRequired,
            v1::QualityOutcome::Unavailable,
            v1::PublicationOutcome::NotRequired,
        ),
        (
            v1::ExecutionOutcome::Cancelled,
            v1::QualityOutcome::Unavailable,
            v1::PublicationOutcome::NotRequired,
        ),
    ] {
        let (mut store, c, attempt) = approved()?;
        let result = worker_result(&attempt, execution, quality, publication);
        store.collect_worker(&attempt.id, None, Some(result.clone()))?;
        store.sync_project_worker_updates()?;
        let updated = store.project_conversation(&c.id)?;
        let evidence = updated.worker_updates.first().ok_or("missing evidence")?;
        let observed = evidence.result.as_ref().ok_or("missing result")?;
        assert_eq!(observed.execution, execution as i32);
        assert_eq!(observed.publication, publication as i32);
        assert_eq!(
            observed.quality.as_ref().ok_or("quality")?.outcome,
            quality as i32
        );
        assert_eq!(evidence.origin.as_ref().ok_or("origin")?.plan_version, 1);
        assert_eq!(evidence.requested_input, result.requested_input);
        assert_eq!(evidence.pull_request, result.pull_request);
        assert_eq!(observed.changed_paths, ["src/lib.rs"]);
        let cursor = updated.update_cursor;
        store.sync_project_worker_updates()?;
        assert_eq!(store.project_conversation(&c.id)?.update_cursor, cursor);
        assert!(store.queued_project_runs()?.is_empty());
        store.validate_durable_state()?;
    }
    Ok(())
}
#[test]
fn retry_origin_and_publication_retry_stay_bound_to_the_approved_version() -> TestResult {
    let (mut store, c, attempt) = approved()?;
    let origin = attempt
        .snapshot
        .as_ref()
        .and_then(|s| s.origin.clone())
        .ok_or("origin")?;
    store.collect_worker(
        &attempt.id,
        None,
        Some(worker_result(
            &attempt,
            v1::ExecutionOutcome::Failed,
            v1::QualityOutcome::Unavailable,
            v1::PublicationOutcome::NotRequired,
        )),
    )?;
    store.sync_project_worker_updates()?;
    let next = store.retry_development_task(
        "retry-evidence",
        &v1::RetryDevelopmentTaskRequest {
            idempotency_key: "retry-evidence".into(),
            development_task_id: attempt.development_task_id.clone(),
        },
    )?;
    assert_eq!(
        next.snapshot.as_ref().and_then(|s| s.origin.as_ref()),
        Some(&origin)
    );
    store.collect_worker(
        &next.id,
        None,
        Some(worker_result(
            &next,
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Passed,
            v1::PublicationOutcome::Failed,
        )),
    )?;
    store.sync_project_worker_updates()?;
    let retry = store.retry_task_publication(
        "retry-publication-evidence",
        &v1::RetryTaskPublicationRequest {
            idempotency_key: "retry-publication-evidence".into(),
            development_task_id: next.development_task_id.clone(),
        },
    )?;
    assert!(retry.publication_only);
    assert_eq!(
        retry.snapshot.as_ref().and_then(|s| s.origin.as_ref()),
        Some(&origin)
    );
    store.collect_worker(
        &retry.id,
        None,
        Some(worker_result(
            &retry,
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Passed,
            v1::PublicationOutcome::Published,
        )),
    )?;
    store.sync_project_worker_updates()?;
    let c = store.project_conversation(&c.id)?;
    assert_eq!(c.worker_updates.len(), 2);
    assert!(c.worker_updates.iter().any(|u| {
        u.attempt_id == retry.id
            && u.publication_only
            && u.result
                .as_ref()
                .is_some_and(|r| r.publication == v1::PublicationOutcome::Published as i32)
    }));
    store.validate_durable_state()?;
    Ok(())
}
#[test]
fn stale_observations_and_restart_do_not_replace_newer_outcomes_or_duplicate_archive_records()
-> TestResult {
    let (mut store, c, attempt) = approved()?;
    let progress = v1::WorkerProgress {
        phase: v1::WorkerPhase::Coding as i32,
        ..Default::default()
    };
    let stale = store.collect_worker(&attempt.id, Some(progress), None)?;
    store.record_project_worker_update(&stale)?;
    let terminal = store.collect_worker(
        &attempt.id,
        None,
        Some(worker_result(
            &attempt,
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Failed,
            v1::PublicationOutcome::Published,
        )),
    )?;
    store.record_project_worker_update(&terminal)?;
    let saved = store.project_conversation(&c.id)?;
    let records = store
        .project_conversation_context(&c.project_id, &c.id)?
        .archive_record_count;
    store.record_project_worker_update(&stale)?;
    let directory = tempdir()?;
    let path = directory.path().join("evidence.sqlite");
    store
        .connection
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    store.sync_project_worker_updates()?;
    assert_eq!(store.project_conversation(&c.id)?, saved);
    assert_eq!(
        store
            .project_conversation_context(&c.project_id, &c.id)?
            .archive_record_count,
        records
    );
    store.validate_durable_state()?;
    Ok(())
}
#[test]
fn evidence_and_history_roll_back_together_when_conversation_save_fails() -> TestResult {
    let (mut store, c, attempt) = approved()?;
    let prior = store.project_conversation_context(&c.project_id, &c.id)?;
    store.connection.execute_batch("CREATE TRIGGER block_evidence BEFORE UPDATE ON project_conversations BEGIN SELECT RAISE(ABORT,'injected'); END;")?;
    assert!(store.sync_project_worker_updates().is_err());
    let count: i64 =
        store
            .connection
            .query_row("SELECT COUNT(*) FROM project_worker_updates", [], |r| {
                r.get(0)
            })?;
    assert_eq!(count, 0);
    assert_eq!(
        store.project_conversation_context(&c.project_id, &c.id)?,
        prior
    );
    store
        .connection
        .execute_batch("DROP TRIGGER block_evidence")?;
    store.sync_project_worker_updates()?;
    assert_eq!(
        store.project_conversation(&c.id)?.worker_updates[0].attempt_id,
        attempt.id
    );
    Ok(())
}
#[test]
fn schema_upgrade_backfills_origins_without_changing_active_execution_authority() -> TestResult {
    let (mut store, c, attempt) = approved()?;
    let mut retained_conversation = store.project_conversation(&c.id)?;
    retained_conversation.runs.push(v1::ProjectAgentRun {
        id: "legacy-outcome-summary".into(),
        state: "queued".into(),
        purpose: "summary".into(),
        ..Default::default()
    });
    store.save_conversation(&mut retained_conversation)?;
    let configuration = ayni_orchestrator_runtime::verification::ReportingConfiguration {
        source: "ayni://orchestrator/supervisors/test".into(),
        endpoint: "http://control-plane/events/verification".into(),
    };
    for table in ["development_tasks", "development_attempts"] {
        let bytes: Vec<u8> = store.connection.query_row(
            &format!("SELECT payload FROM {table} LIMIT 1"),
            [],
            |r| r.get(0),
        )?;
        if table == "development_tasks" {
            let mut t: v1::DevelopmentTask = decode(&bytes)?;
            t.snapshot.as_mut().ok_or("snapshot")?.origin = None;
            store.connection.execute(
                "UPDATE development_tasks SET payload=?1 WHERE id=?2",
                params![t.encode_to_vec(), t.id],
            )?;
        } else {
            let mut a: v1::DevelopmentAttempt = decode(&bytes)?;
            a.snapshot.as_mut().ok_or("snapshot")?.origin = None;
            store.connection.execute(
                "UPDATE development_attempts SET payload=?1 WHERE id=?2",
                params![a.encode_to_vec(), a.id],
            )?;
        }
    }
    let admission =
        store.select_coding_execution(&attempt.id, "execution-one", Some(configuration))?;
    store.connection.execute_batch("DROP TABLE project_worker_updates; DROP TABLE project_task_origins; UPDATE clean_schema SET version=23;")?;
    let directory = tempdir()?;
    let path = directory.path().join("legacy.sqlite");
    store
        .connection
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(
        serde_json::to_value(store.coding_admission(&attempt.id)?)?,
        serde_json::to_value(admission)?
    );
    assert_eq!(
        store
            .development_attempt(&attempt.id)?
            .snapshot
            .and_then(|s| s.origin)
            .ok_or("origin")?
            .conversation_id,
        c.id
    );
    assert!(store.queued_project_runs()?.is_empty());
    store.sync_project_worker_updates()?;
    store.validate_durable_state()?;
    Ok(())
}

#[test]
fn duplicate_and_reordered_coding_deliveries_return_only_the_latest_accepted_evidence() -> TestResult
{
    use ayni_orchestrator_events::{
        EventIdentity,
        v1::{CodingReportAvailable, CodingReportKind},
    };
    let (mut store, c, attempt) = approved()?;
    let source = "ayni://orchestrator/supervisors/evidence";
    let admission = store
        .select_coding_execution(
            &attempt.id,
            "evidence-execution",
            Some(
                ayni_orchestrator_runtime::verification::ReportingConfiguration {
                    source: source.into(),
                    endpoint: "http://control-plane/events/verification".into(),
                },
            ),
        )?
        .ok_or("admission")?;
    store.bind_coding_request(&attempt.id, &admission.binding.generation, DIGEST)?;
    let mut binding = admission.binding;
    binding.pod_uid = "evidence-pod".into();
    binding.request_digest = DIGEST.into();
    let report = |sequence, kind| CodingReportAvailable {
        binding: Some(binding.clone()),
        sequence,
        kind,
        sha256: DIGEST.into(),
        size_bytes: 100,
        contract: ayni_orchestrator_events::coding::CONTRACT.into(),
    };
    let identity = |id: &str| EventIdentity {
        source: source.into(),
        id: id.into(),
    };
    let usage = v1::AttemptModelUsage {
        collection_status: v1::ModelUsageCollectionStatus::Unavailable as i32,
        ..Default::default()
    };
    let initial = report(1, CodingReportKind::Progress as i32);
    let initial_id = identity("00000000-0000-4000-8000-000000000001");
    store.receive_coding(&initial_id, &initial)?;
    assert!(store.commit_coding_report(
        &initial_id,
        &initial,
        Some(v1::WorkerProgress {
            phase: v1::WorkerPhase::Coding as i32,
            ..Default::default()
        }),
        None,
        vec![],
        usage.clone()
    )?);
    store.sync_project_worker_updates()?;
    let final_report = report(257, CodingReportKind::Final as i32);
    let final_id = identity("00000000-0000-4000-8000-000000000002");
    store.receive_coding(&final_id, &final_report)?;
    assert!(store.commit_coding_report(
        &final_id,
        &final_report,
        None,
        Some(worker_result(
            &attempt,
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Failed,
            v1::PublicationOutcome::Published
        )),
        vec![],
        usage.clone()
    )?);
    store.sync_project_worker_updates()?;
    let saved = store.project_conversation(&c.id)?;
    assert!(!store.commit_coding_report(
        &final_id,
        &final_report,
        None,
        Some(worker_result(
            &attempt,
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Failed,
            v1::PublicationOutcome::Published
        )),
        vec![],
        usage.clone()
    )?);
    let delayed = report(2, CodingReportKind::Progress as i32);
    let delayed_id = identity("00000000-0000-4000-8000-000000000003");
    store.receive_coding(&delayed_id, &delayed)?;
    assert!(!store.commit_coding_report(
        &delayed_id,
        &delayed,
        Some(v1::WorkerProgress {
            phase: v1::WorkerPhase::Checkout as i32,
            ..Default::default()
        }),
        None,
        vec![],
        usage
    )?);
    store.sync_project_worker_updates()?;
    assert_eq!(store.project_conversation(&c.id)?, saved);
    assert_eq!(saved.worker_updates.len(), 1);
    store.validate_durable_state()?;
    Ok(())
}

#[test]
fn missing_or_mismatched_retained_evidence_cannot_become_passing_planner_context() -> TestResult {
    let (mut store, c, attempt) = approved()?;
    // Historical retained state can predate current evidence validation.
    let mut result = worker_result(
        &attempt,
        v1::ExecutionOutcome::Succeeded,
        v1::QualityOutcome::Passed,
        v1::PublicationOutcome::Published,
    );
    result
        .quality
        .as_mut()
        .ok_or("quality")?
        .evidence
        .as_mut()
        .ok_or("evidence")?
        .source_fingerprint = String::new();
    let mut retained = attempt.clone();
    retained.result = Some(result);
    retained.resource_version += 1;
    retained.state = v1::DevelopmentAttemptState::Succeeded as i32;
    let task = store.development_task(&attempt.development_task_id)?;
    persist_attempt_update(
        &store.connection,
        &retained,
        attempt.resource_version,
        &task,
    )?;
    store.sync_project_worker_updates()?;
    let c = store.project_conversation(&c.id)?;
    assert_eq!(
        c.worker_updates[0]
            .result
            .as_ref()
            .and_then(|r| r.quality.as_ref())
            .ok_or("quality")?
            .outcome,
        v1::QualityOutcome::Unavailable as i32
    );
    let context = super::super::super::project_context::safe_outcome(&task.id, &retained);
    assert_eq!(
        context["quality_state"],
        v1::QualityOutcome::Unavailable as i32
    );
    Ok(())
}

#[path = "project_worker_evidence_fixture.rs"]
mod fixture;
