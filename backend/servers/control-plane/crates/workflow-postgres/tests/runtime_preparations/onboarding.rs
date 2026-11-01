use super::*;

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
#[allow(clippy::too_many_lines)]
async fn repository_onboarding_retains_one_grant_and_recovers_fenced_steps() -> TestResult {
    use ayni_orchestrator_workflow_postgres::store::repository_runtime::Settings;
    let f = Fixture::new().await?;
    let settings = Settings {
        repository_id: f.repository.clone(),
        expected_connection_revision: 1,
        registry_connection_id: f.registry.clone(),
        signer_id: f.signer.clone(),
        platform: "linux/arm64".into(),
        max_seconds: 3600,
        max_builds: 1,
        allow_build: true,
        allow_replace: false,
    };
    let command = key();
    let original = f
        .store
        .start_repository_runtime(&command, settings.clone())
        .await?;
    assert_eq!(
        f.store
            .start_repository_runtime(&command, settings.clone())
            .await?,
        original
    );
    assert!(matches!(
        f.store
            .start_repository_runtime(
                &command,
                Settings {
                    max_builds: 2,
                    ..settings.clone()
                }
            )
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        f.store
            .start_repository_runtime(&key(), settings.clone())
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    let first = f
        .store
        .claim_repository_runtime()
        .await?
        .ok_or("missing claim")?;
    assert_eq!(first.run.id, original.id);
    assert!(f.store.claim_repository_runtime().await?.is_none());
    let (admin, driver) = ayni_orchestrator_workflow_postgres::connect(&std::env::var(
        "AYNI_POSTGRES_FEASIBILITY_URL",
    )?)
    .await?;
    admin.execute("UPDATE workflow_repository_runtime_runs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",&[&original.id]).await?;
    let restarted = WorkflowStore::new(std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?);
    let claim = restarted
        .claim_repository_runtime()
        .await?
        .ok_or("missing recovered claim")?;
    assert_ne!(first.token, claim.token);
    assert!(matches!(
        f.store
            .advance_repository_runtime(&first, "inspecting", &"a".repeat(40))
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    restarted
        .advance_repository_runtime(&claim, "inspecting", &"a".repeat(40))
        .await?;
    let inspection_claim = restarted
        .claim_repository_runtime()
        .await?
        .ok_or("inspection claim")?;
    assert_eq!(inspection_claim.run.commit_sha, "a".repeat(40));
    let observed = f.store.get_runtime_inspection_by_id(&f.inspection).await?;
    let inspected = restarted
        .begin_repository_inspection(Inspect {
            key: &original.inspection_key,
            repository_id: &f.repository,
            expected_connection_revision: 1,
            commit_sha: &inspection_claim.run.commit_sha,
        })
        .await?;
    restarted
        .finish_repository_inspection(
            &original.inspection_key,
            Ok(observed.result.ok_or("fixture result")?),
        )
        .await?;
    restarted
        .advance_repository_runtime(&inspection_claim, "admitting", &inspected.id)
        .await?;
    let admission_claim = restarted
        .claim_repository_runtime()
        .await?
        .ok_or("admission claim")?;
    let input = Start {
        inspection_id: &inspected.id,
        ..f.input(&original.preparation_key)
    };
    assert!(matches!(
        restarted
            .start_reviewed_runtime_preparation(input.clone(), f.freeze.clone(), Some((2, 1)))
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    let operation = restarted
        .start_reviewed_runtime_preparation(input.clone(), f.freeze.clone(), Some((1, 1)))
        .await?;
    // Crash after admission: its deterministic key recovers the same original operation.
    assert_eq!(
        restarted
            .replay_runtime_start(&input)
            .await?
            .ok_or("start receipt")?
            .id,
        operation.id
    );
    restarted
        .advance_repository_runtime(&admission_claim, "completed", &operation.id)
        .await?;
    assert_eq!(
        restarted
            .latest_repository_runtime(&f.repository)
            .await?
            .ok_or("latest")?
            .preparation_id,
        operation.id
    );
    assert_eq!(
        restarted.repository_runtime_receipt(&command).await?,
        original
    );
    assert!(matches!(
        restarted.start_repository_runtime(&key(), settings).await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    // Runtime role cannot alter the reviewed request or remove its history.
    let (runtime, runtime_driver) = ayni_orchestrator_workflow_postgres::connect(&std::env::var(
        "AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL",
    )?)
    .await?;
    assert!(
        runtime
            .execute(
                "UPDATE workflow_repository_runtime_runs SET safe_request='{}' WHERE id=$1",
                &[&original.id]
            )
            .await
            .is_err()
    );
    assert!(
        runtime
            .execute(
                "DELETE FROM workflow_repository_runtime_runs WHERE id=$1",
                &[&original.id]
            )
            .await
            .is_err()
    );
    driver.abort();
    runtime_driver.abort();
    Ok(())
}
