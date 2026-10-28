use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore,
    store::{
        connections::SaveGithub,
        repositories::{Connect, ObservedRepository, Repository},
    },
};
use uuid::Uuid;
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
fn key() -> String {
    Uuid::now_v7().to_string()
}
fn provider_id() -> u64 {
    u64::try_from(Uuid::new_v4().as_u128() % 1_000_000_000_000).unwrap_or(1) + 1
}
fn observed(id: u64) -> ObservedRepository {
    ObservedRepository {
        provider_repository_id: id,
        full_name: "fixture/shared".into(),
        default_branch: "main".into(),
        private: true,
    }
}
const fn input<'a>(key: &'a str, connection: &'a str, revision: u64, id: u64) -> Connect<'a> {
    Connect {
        key,
        connection_id: connection,
        expected_revision: revision,
        installation_id: 123,
        provider_repository_id: id,
    }
}
async fn source(store: &WorkflowStore) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let op = store
        .save_github_connection(SaveGithub {
            key: &key(),
            id: "",
            expected: 0,
            name: "Catalog fixture",
            app_id: "123",
            private_key_pem: "repository-fixture-not-a-real-key",
        })
        .await?;
    store.finish_github_validation(&op.id, "").await?;
    Ok(op.connection_id)
}
#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn repository_catalog_preserves_exact_receipts_and_access_boundaries() -> TestResult {
    ayni_orchestrator_workflow_postgres::migrate_for_runtime(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let runtime_url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    ayni_orchestrator_workflow_postgres::validate_for_runtime(
        &runtime_url,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let store = WorkflowStore::new(runtime_url.clone());
    hierarchy_renames(&store).await?;
    let connection = source(&store).await?;
    let repo = exact_retries(&store, &runtime_url, &connection).await?;
    associations(&store, &repo, &connection).await?;
    rotation_and_competing_sources(&store, &connection, repo.provider_repository_id).await?;
    privileges(&runtime_url, &repo).await?;
    Ok(())
}
async fn exact_retries(
    store: &WorkflowStore,
    url: &str,
    connection: &str,
) -> Result<Repository, Box<dyn std::error::Error + Send + Sync>> {
    let command = key();
    let id = provider_id();
    let request = input(&command, connection, 1, id);
    let accepted = store.begin_repository_connect(request.clone()).await?;
    assert_eq!(accepted.state, "accepted");
    let retry = WorkflowStore::new(url.to_owned())
        .begin_repository_connect(request.clone())
        .await?;
    assert_eq!(retry.id, accepted.id);
    assert!(matches!(
        store
            .begin_repository_connect(input(&command, connection, 1, id + 1))
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    let unavailable = store
        .finish_repository_connect(&command, Err("github_unavailable"))
        .await?;
    assert_eq!(unavailable.state, "accepted");
    assert_eq!(unavailable.safe_reason, "github_unavailable");
    let mut wrong = observed(id);
    wrong.provider_repository_id += 1;
    assert!(matches!(
        store.finish_repository_connect(&command, Ok(wrong)).await,
        Err(StoreError::InvalidInput(_))
    ));
    let (a, b) = tokio::join!(
        store.finish_repository_connect(&command, Ok(observed(id))),
        store.finish_repository_connect(&command, Ok(observed(id)))
    );
    let a = a?;
    let b = b?;
    assert_eq!(a.repository, b.repository);
    assert_eq!(a.state, "completed");
    let repo = a.repository.clone().ok_or("missing accepted repository")?;
    assert_eq!(
        store.get_repository_operation(&command).await?.repository,
        Some(repo.clone())
    );
    assert_eq!(
        store.begin_repository_connect(request).await?.repository,
        Some(repo.clone())
    );
    assert_eq!(
        store
            .finish_repository_connect(&command, Err("github_access_denied"))
            .await?
            .state,
        "completed"
    );
    let serialized = serde_json::to_string(&a)?;
    assert!(!serialized.contains("repository-fixture-not-a-real-key"));
    assert!(!serialized.contains("credential_revision"));
    let second = key();
    store
        .begin_repository_connect(input(&second, connection, 1, id))
        .await?;
    assert_eq!(
        store
            .finish_repository_connect(&second, Ok(observed(id)))
            .await?
            .repository,
        Some(repo.clone())
    );
    let (_, next) = store.list_repository_operations("", 1).await?;
    assert!(!next.is_empty());
    assert!(store.list_repositories(None, "", 101).await.is_err());
    assert!(
        store
            .list_repository_operations("invalid", 50)
            .await
            .is_err()
    );
    Ok(repo)
}
async fn associations(store: &WorkflowStore, repo: &Repository, connection: &str) -> TestResult {
    let a = store.create_workspace(&key(), "Catalog A").await?.resource;
    let b = store.create_workspace(&key(), "Catalog B").await?.resource;
    let pa = store
        .create_project(&key(), &a.id, "A project", "Use shared source")
        .await?
        .resource;
    let pb = store
        .create_project(&key(), &b.id, "B project", "Reuse shared source")
        .await?
        .resource;
    let command = key();
    let first = store
        .associate_repository(&command, &a.id, &pa.id, &repo.id)
        .await?;
    let retry = store
        .associate_repository(&command, &a.id, &pa.id, &repo.id)
        .await?;
    assert_eq!(first, retry);
    assert!(matches!(
        store
            .associate_repository(&command, &b.id, &pb.id, &repo.id)
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    store
        .associate_repository(&key(), &b.id, &pb.id, &repo.id)
        .await?;
    for (w, p) in [(&a.id, &pa.id), (&b.id, &pb.id)] {
        let (items, next) = store.list_repositories(Some((w, p)), "", 50).await?;
        assert_eq!(items, vec![repo.clone()]);
        assert!(next.is_empty());
    }
    assert!(matches!(
        store.list_repositories(Some((&a.id, &pb.id)), "", 50).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store
            .associate_repository(&key(), &a.id, &pb.id, &repo.id)
            .await,
        Err(StoreError::NotFound)
    ));
    let pending = key();
    let id = provider_id();
    store
        .begin_repository_connect(input(&pending, connection, 1, id))
        .await?;
    store
        .set_connection_enabled(&key(), connection, 1, false)
        .await?;
    let fenced = store
        .finish_repository_connect(&pending, Ok(observed(id)))
        .await?;
    assert_eq!(fenced.state, "superseded");
    assert!(fenced.repository.is_none());
    assert!(matches!(
        store
            .begin_repository_connect(input(&key(), connection, 2, id))
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    assert!(matches!(
        store
            .associate_repository(&key(), &a.id, &pa.id, &repo.id)
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    assert_eq!(
        store
            .associate_repository(&command, &a.id, &pa.id, &repo.id)
            .await?,
        first
    );
    store
        .set_connection_enabled(&key(), connection, 2, true)
        .await?;
    Ok(())
}
async fn rotation_and_competing_sources(
    store: &WorkflowStore,
    connection: &str,
    id: u64,
) -> TestResult {
    let command = key();
    let other = provider_id();
    store
        .begin_repository_connect(input(&command, connection, 3, other))
        .await?;
    let rotation = store
        .save_github_connection(SaveGithub {
            key: &key(),
            id: connection,
            expected: 3,
            name: "Rotated source",
            app_id: "123",
            private_key_pem: "replacement-fixture",
        })
        .await?;
    assert!(matches!(
        store.repository_source(connection, 3).await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    assert_eq!(
        store
            .finish_repository_connect(&command, Ok(observed(other)))
            .await?
            .state,
        "superseded"
    );
    store.finish_github_validation(&rotation.id, "").await?;
    assert!(store.repository_source(connection, 4).await.is_ok());
    let competing = source(store).await?;
    let command = key();
    store
        .begin_repository_connect(input(&command, &competing, 1, id))
        .await?;
    let result = store
        .finish_repository_connect(&command, Ok(observed(id)))
        .await?;
    assert_eq!(result.state, "failed");
    assert_eq!(result.safe_reason, "repository_already_connected");
    assert!(
        store
            .finish_repository_connect(&command, Err("unsafe-provider-text"))
            .await
            .is_err()
    );
    Ok(())
}
async fn privileges(url: &str, repo: &Repository) -> TestResult {
    let (client, driver) = tokio_postgres::connect(url, tokio_postgres::NoTls).await?;
    let task = tokio::spawn(driver);
    for sql in [
        "UPDATE workflow_repositories SET full_name='rewritten'",
        "DELETE FROM workflow_repositories",
        "UPDATE workflow_repository_operations SET safe_request='rewritten'",
        "UPDATE workflow_project_repositories SET workspace_id='rewritten'",
        "DELETE FROM workflow_repository_operations",
        "DELETE FROM workflow_repository_association_receipts",
    ] {
        let error = client
            .execute(sql, &[])
            .await
            .err()
            .ok_or("unexpected mutation privilege")?;
        assert_eq!(
            error.code(),
            Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
        );
    }
    let rows=client.query("SELECT workspace_id,project_id FROM workflow_project_repositories WHERE repository_id=$1 ORDER BY workspace_id",&[&repo.id]).await?;
    let wrong_workspace: String = rows[0].get(0);
    let wrong_project: String = rows[1].get(1);
    let error=client.execute("INSERT INTO workflow_project_repositories(workspace_id,project_id,repository_id) VALUES ($1,$2,$3)",&[&wrong_workspace,&wrong_project,&repo.id]).await.err().ok_or("cross-scope association succeeded")?;
    assert_eq!(
        error.code(),
        Some(&tokio_postgres::error::SqlState::FOREIGN_KEY_VIOLATION)
    );
    task.abort();
    Ok(())
}

async fn hierarchy_renames(store: &WorkflowStore) -> TestResult {
    let workspace = store.create_workspace(&key(), "Before").await?.resource;
    let other = store.create_workspace(&key(), "Other").await?.resource;
    let project = store
        .create_project(&key(), &workspace.id, "Original", "Keep goal")
        .await?
        .resource;
    let receipt_key = key();
    let receipt = store
        .rename_workspace(&receipt_key, &workspace.id, 1, "Renamed")
        .await?;
    assert_eq!(
        store
            .rename_workspace(&receipt_key, &workspace.id, 1, "Renamed")
            .await?,
        receipt
    );
    assert!(matches!(
        store
            .rename_workspace(&receipt_key, &workspace.id, 1, "Different")
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        store
            .rename_workspace(&key(), &workspace.id, 1, "Stale")
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    store
        .rename_workspace(&key(), &workspace.id, 2, "Latest")
        .await?;
    assert_eq!(
        store
            .rename_workspace(&receipt_key, &workspace.id, 1, "Renamed")
            .await?,
        receipt
    );
    assert!(
        store
            .rename_project(&key(), &other.id, &project.id, 1, "Wrong scope")
            .await
            .is_err()
    );
    let project_key = key();
    let project_receipt = store
        .rename_project(&project_key, &workspace.id, &project.id, 1, "Updated")
        .await?;
    assert_eq!(
        store
            .rename_project(&project_key, &workspace.id, &project.id, 1, "Updated")
            .await?,
        project_receipt
    );
    assert!(matches!(
        store
            .rename_project(&key(), &workspace.id, &project.id, 1, "Stale")
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    Ok(())
}
