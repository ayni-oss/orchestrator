use ayni_orchestrator_workflow_postgres::store::{
    StoreError, WorkflowStore,
    connections::SaveGithub,
    repositories::{Connect, ObservedRepository},
    repository_inspections::{Finding, Inspect, InspectionResult},
};
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
fn key() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn observation() -> InspectionResult {
    InspectionResult {
        policy_digest: "sha256:policy".into(),
        lock_digest: "sha256:lock".into(),
        lock_fingerprint: "sha256:fingerprint".into(),
        ayni_version: "1.0.0".into(),
        lock_schema: "v1".into(),
        tools: vec!["cargo".into()],
        trust_key_ids: vec!["required-key".into()],
        findings: vec![Finding {
            code: "trust_required".into(),
            path: "ayni.lock".into(),
            action: "Configure the required trust key.".into(),
        }],
    }
}
#[allow(clippy::too_many_lines)]
#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn frozen_inspections_preserve_receipts_fence_source_changes_and_retain_history() -> TestResult
{
    let role = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?;
    ayni_orchestrator_workflow_postgres::migrate_for_runtime(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        &role,
    )
    .await?;
    let url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    ayni_orchestrator_workflow_postgres::validate_for_runtime(&url, &role).await?;
    let store = WorkflowStore::new(url.clone());
    let source = store
        .save_github_connection(SaveGithub {
            key: &key(),
            id: "",
            expected: 0,
            name: "Inspection source",
            app_id: "123",
            private_key_pem: "inspection-fixture-not-a-real-key",
        })
        .await?;
    store.finish_github_validation(&source.id, "").await?;
    let provider_id = u64::try_from(uuid::Uuid::new_v4().as_u128() % 1_000_000_000_000)? + 1;
    let connect = key();
    store
        .begin_repository_connect(Connect {
            key: &connect,
            connection_id: &source.connection_id,
            expected_revision: 1,
            installation_id: 123,
            provider_repository_id: provider_id,
        })
        .await?;
    let repo = store
        .finish_repository_connect(
            &connect,
            Ok(ObservedRepository {
                provider_repository_id: provider_id,
                full_name: "fixture/inspection".into(),
                default_branch: "main".into(),
                private: true,
            }),
        )
        .await?
        .repository
        .ok_or("repository missing")?;
    assert_eq!(store.inspection_repository(&repo.id).await?, repo);
    let command = key();
    let commit = "a".repeat(40);
    let request = Inspect {
        key: &command,
        repository_id: &repo.id,
        expected_connection_revision: 1,
        commit_sha: &commit,
    };
    let accepted = store.begin_repository_inspection(request.clone()).await?;
    assert_eq!(accepted.state, "accepted");
    assert_eq!(
        WorkflowStore::new(url.clone())
            .begin_repository_inspection(request.clone())
            .await?,
        accepted
    );
    assert!(matches!(
        store
            .begin_repository_inspection(Inspect {
                commit_sha: &"b".repeat(40),
                ..request.clone()
            })
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        store
            .begin_repository_inspection(Inspect {
                key: &key(),
                commit_sha: "main",
                ..request.clone()
            })
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    for reason in [
        "github_unavailable",
        "github_rate_limited",
        "inspection_unavailable",
    ] {
        let retry = store
            .finish_repository_inspection(&command, Err(reason))
            .await?;
        assert_eq!(retry.state, "accepted");
        assert_eq!(retry.safe_reason, reason);
    }
    assert!(
        store
            .finish_repository_inspection(&command, Err("raw credential-bearing failure"))
            .await
            .is_err()
    );
    let (a, b) = tokio::join!(
        store.finish_repository_inspection(&command, Ok(observation())),
        store.finish_repository_inspection(&command, Ok(observation()))
    );
    let completed = a?;
    assert_eq!(completed, b?);
    assert_eq!(completed.state, "completed");
    assert_eq!(completed.result, Some(observation()));
    assert_eq!(completed.accepted_at_unix_ms, accepted.accepted_at_unix_ms);
    assert_eq!(
        store
            .finish_repository_inspection(&command, Err("github_access_denied"))
            .await?,
        completed
    );
    assert_eq!(
        store.begin_repository_inspection(request.clone()).await?,
        completed
    );
    let pending = key();
    store
        .begin_repository_inspection(Inspect {
            key: &pending,
            ..request.clone()
        })
        .await?;
    let failed = key();
    store
        .begin_repository_inspection(Inspect {
            key: &failed,
            ..request.clone()
        })
        .await?;
    assert_eq!(
        store
            .finish_repository_inspection(&failed, Err("github_commit_unavailable"))
            .await?
            .state,
        "failed"
    );
    assert_eq!(
        store
            .finish_repository_inspection(&failed, Ok(observation()))
            .await?
            .state,
        "failed"
    );
    store
        .set_connection_enabled(&key(), &source.connection_id, 1, false)
        .await?;
    let stale = store
        .finish_repository_inspection(&pending, Ok(observation()))
        .await?;
    assert_eq!(stale.state, "superseded");
    assert_eq!(stale.safe_reason, "github_source_changed");
    assert!(stale.result.is_none());
    assert!(matches!(
        store
            .begin_repository_inspection(Inspect {
                key: &key(),
                expected_connection_revision: 2,
                ..request.clone()
            })
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    assert_eq!(store.get_repository_inspection(&command).await?, completed);
    let (page, next) = store.list_repository_inspections(&repo.id, "", 1).await?;
    assert_eq!(page.len(), 1);
    assert!(!next.is_empty());
    let (rest, end) = store
        .list_repository_inspections(&repo.id, &next, 100)
        .await?;
    assert_eq!(rest.len(), 2);
    assert!(end.is_empty());
    assert!(rest.iter().all(|v| v.id != page[0].id));
    assert!(
        store
            .list_repository_inspections(&repo.id, "", 101)
            .await
            .is_err()
    );
    store
        .set_connection_enabled(&key(), &source.connection_id, 2, true)
        .await?;
    let rotation_pending = key();
    store
        .begin_repository_inspection(Inspect {
            key: &rotation_pending,
            expected_connection_revision: 3,
            ..request
        })
        .await?;
    store
        .save_github_connection(SaveGithub {
            key: &key(),
            id: &source.connection_id,
            expected: 3,
            name: "Rotated source",
            app_id: "123",
            private_key_pem: "rotated-inspection-fixture",
        })
        .await?;
    assert_eq!(
        store
            .finish_repository_inspection(&rotation_pending, Ok(observation()))
            .await?
            .state,
        "superseded"
    );
    let safe = serde_json::to_string(&completed)?;
    assert!(!safe.contains("credential_revision"));
    assert!(!safe.contains("inspection-fixture"));
    let (client, driver) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await?;
    let task = tokio::spawn(driver);
    for sql in [
        "DELETE FROM workflow_repository_inspections",
        "UPDATE workflow_repository_inspections SET commit_sha=repeat('b',40)",
        "UPDATE workflow_repository_inspections SET safe_request='changed'",
        "UPDATE workflow_repository_inspections SET repository_id='changed'",
        "UPDATE workflow_repository_inspections SET accepted_at_unix_ms=0",
    ] {
        let error = client
            .execute(sql, &[])
            .await
            .err()
            .ok_or("unexpected immutable receipt mutation")?;
        assert_eq!(
            error.code(),
            Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
        );
    }
    task.abort();
    Ok(())
}
