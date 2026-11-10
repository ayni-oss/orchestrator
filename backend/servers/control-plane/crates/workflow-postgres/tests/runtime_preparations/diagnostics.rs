use super::{
    Binding, Error, Evidence, Fixture, Operation, StageReport, TestResult, WorkflowStore, hash, key,
};

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn certification_diagnostics_replay_after_cleanup_and_store_restart() -> TestResult {
    use ayni_orchestrator_events::execution_diagnostics::{
        CERTIFICATION_DIAGNOSTIC, CertificationDiagnostic, Diagnostic, Kind,
    };
    let f = Fixture::new().await?;
    let (op, binding) = reach_certification(&f).await?;
    let cause = CertificationDiagnostic::ProtectedContentRootMismatch;
    let mut evidence = Evidence::default();
    evidence.details.insert(
        CERTIFICATION_DIAGNOSTIC.into(),
        serde_json::to_string(&cause)?,
    );
    let report = StageReport {
        binding: binding.clone(),
        outcome: "failed".into(),
        safe_reason: "certification_failed".into(),
        evidence,
    };
    f.store.report_runtime_stage(&report).await?;
    f.store.report_runtime_stage(&report).await?;
    let mut lifecycle = Diagnostic::new(
        Kind::Runtime,
        &op.id,
        binding.generation,
        "certify",
        "retiring",
    );
    lifecycle.outcome = "failed".into();
    lifecycle.pod_uid = key();
    assert!(
        f.store
            .record_execution_lifecycle(&lifecycle)
            .await
            .is_err()
    );
    lifecycle.pod_uid = binding.pod_uid.clone();
    f.store.record_execution_lifecycle(&lifecycle).await?;
    f.store.record_execution_lifecycle(&lifecycle).await?;
    f.store.stop_runtime_stage(&binding).await?;
    let cleaned = f
        .cleanup(&f.store.get_runtime_preparation(&op.id).await?)
        .await?;
    assert!(cleaned.evidence.cleanup_complete);

    let restarted = WorkflowStore::new(std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?);
    let records = restarted
        .list_execution_diagnostics(Kind::Runtime, &op.id, 0)
        .await?;
    let failure = records
        .iter()
        .find(|entry| entry.diagnostic.certification == Some(cause))
        .ok_or("certification diagnostic missing after cleanup")?;
    assert_eq!(
        records
            .iter()
            .filter(|entry| entry.diagnostic.certification == Some(cause))
            .count(),
        1
    );
    assert!(
        records
            .iter()
            .any(|entry| entry.diagnostic.outcome == "build_image")
    );
    let resumed = restarted
        .list_execution_diagnostics(Kind::Runtime, &op.id, failure.cursor)
        .await?;
    assert_eq!(
        resumed
            .iter()
            .filter(|entry| entry.diagnostic == lifecycle)
            .count(),
        1
    );
    assert!(resumed.iter().all(|entry| entry.cursor > failure.cursor));
    assert!(
        restarted
            .list_execution_diagnostics(Kind::Task, &op.id, 0)
            .await?
            .is_empty()
    );
    restarted
        .mark_execution_diagnostic_published(failure.cursor)
        .await?;
    assert_eq!(
        restarted
            .list_execution_diagnostics(Kind::Runtime, &op.id, 0)
            .await?,
        records
    );
    Ok(())
}
async fn reach_certification(f: &Fixture) -> Result<(Operation, Binding), Error> {
    let op = f.start().await?;
    let op = f.discover(&op).await?;
    let op = f.finish(&op, "missing", Evidence::default()).await?;
    let build = f.bind(&op).await?;
    f.store
        .record_runtime_build_checkpoint(
            &build,
            ayni_orchestrator_events::runtime_preparation::BuildMilestone::BuildImage,
        )
        .await?;
    f.store
        .report_runtime_stage(&StageReport {
            binding: build.clone(),
            outcome: "completed".into(),
            safe_reason: String::new(),
            evidence: Evidence {
                artifact_digest: hash("manifest"),
                ..Evidence::default()
            },
        })
        .await?;
    f.store.stop_runtime_stage(&build).await?;
    let op = f.store.get_runtime_preparation(&op.id).await?;
    let op = f
        .finish(
            &op,
            "completed",
            Evidence {
                artifact_digest: hash("manifest"),
                unsigned_manifest_digest: hash("manifest"),
                protected_content_root: hash("protected"),
                ..Evidence::default()
            },
        )
        .await?;
    assert_eq!(op.stage, "certify");
    let binding = f.bind(&op).await?;
    Ok((op, binding))
}
