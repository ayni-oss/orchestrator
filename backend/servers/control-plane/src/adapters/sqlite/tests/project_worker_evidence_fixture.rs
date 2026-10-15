//! Synthetic retained acceptance states for the disposable Docker Desktop reconciliation gate.
use super::*;
#[test]
#[ignore = "exports synthetic accepted worker evidence for disposable Docker Desktop validation"]
fn export_worker_evidence_fixtures() -> TestResult {
    let directory = std::env::var("AYNI_WORKER_EVIDENCE_FIXTURE_DIR")?;
    for (name, execution, quality, publication) in [
        (
            "success",
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Passed,
            v1::PublicationOutcome::Published,
        ),
        (
            "failed-quality",
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Failed,
            v1::PublicationOutcome::Published,
        ),
        (
            "unavailable",
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Unavailable,
            v1::PublicationOutcome::Published,
        ),
        (
            "question",
            v1::ExecutionOutcome::InputRequired,
            v1::QualityOutcome::Unavailable,
            v1::PublicationOutcome::NotRequired,
        ),
        (
            "cancelled",
            v1::ExecutionOutcome::Cancelled,
            v1::QualityOutcome::Unavailable,
            v1::PublicationOutcome::NotRequired,
        ),
        (
            "publication-failed",
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Passed,
            v1::PublicationOutcome::Failed,
        ),
        (
            "retry",
            v1::ExecutionOutcome::Succeeded,
            v1::QualityOutcome::Passed,
            v1::PublicationOutcome::Published,
        ),
        (
            "progress",
            v1::ExecutionOutcome::Unspecified,
            v1::QualityOutcome::Unspecified,
            v1::PublicationOutcome::Unspecified,
        ),
    ] {
        let (mut store, c, mut attempt) = approved()?;
        if name == "retry" {
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
            attempt = store.retry_development_task(
                "fixture-retry",
                &v1::RetryDevelopmentTaskRequest {
                    idempotency_key: "fixture-retry".into(),
                    development_task_id: attempt.development_task_id.clone(),
                },
            )?;
            store.collect_worker(
                &attempt.id,
                None,
                Some(worker_result(
                    &attempt,
                    v1::ExecutionOutcome::Succeeded,
                    v1::QualityOutcome::Passed,
                    v1::PublicationOutcome::Failed,
                )),
            )?;
            attempt = store.retry_task_publication(
                "fixture-publication",
                &v1::RetryTaskPublicationRequest {
                    idempotency_key: "fixture-publication".into(),
                    development_task_id: attempt.development_task_id.clone(),
                },
            )?;
        }
        if name == "progress" {
            store.collect_worker(
                &attempt.id,
                Some(v1::WorkerProgress {
                    phase: v1::WorkerPhase::Checking as i32,
                    ..Default::default()
                }),
                None,
            )?;
        } else {
            store.collect_worker(
                &attempt.id,
                None,
                Some(worker_result(&attempt, execution, quality, publication)),
            )?;
        }
        store.validate_durable_state()?;
        let manifest = serde_json::json!({"conversationId":c.id,"projectId":c.project_id,"taskId":attempt.development_task_id,"attemptId":attempt.id,"updates":if name=="retry"{2}else{1},"execution":execution.as_str_name(),"quality":quality.as_str_name(),"publication":publication.as_str_name()});
        let path = std::path::Path::new(&directory).join(format!("{name}.sqlite"));
        store
            .connection
            .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
        std::fs::write(
            path.with_extension("json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        if name == "success" {
            for (table, id) in [
                ("development_tasks", attempt.development_task_id.clone()),
                ("development_attempts", attempt.id.clone()),
            ] {
                let bytes: Vec<u8> = store.connection.query_row(
                    &format!("SELECT payload FROM {table} WHERE id=?1"),
                    [&id],
                    |r| r.get(0),
                )?;
                let payload = if table == "development_tasks" {
                    let mut t: v1::DevelopmentTask = decode(&bytes)?;
                    t.snapshot.as_mut().ok_or("snapshot")?.origin = None;
                    t.encode_to_vec()
                } else {
                    let mut a: v1::DevelopmentAttempt = decode(&bytes)?;
                    a.snapshot.as_mut().ok_or("snapshot")?.origin = None;
                    a.encode_to_vec()
                };
                store.connection.execute(
                    &format!("UPDATE {table} SET payload=?1 WHERE id=?2"),
                    params![payload, id],
                )?;
            }
            store.connection.execute_batch("DROP TABLE project_worker_updates; DROP TABLE project_task_origins; UPDATE clean_schema SET version=23;")?;
            let legacy = std::path::Path::new(&directory).join("legacy.sqlite");
            store
                .connection
                .execute("VACUUM INTO ?1", [legacy.to_string_lossy().as_ref()])?;
            std::fs::write(
                legacy.with_extension("json"),
                serde_json::to_vec_pretty(&manifest)?,
            )?;
        }
    }
    Ok(())
}
