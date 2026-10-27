use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, store::connections::SaveGithub,
};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
fn key() -> String {
    Uuid::now_v7().to_string()
}
const fn candidate<'a>(key: &'a str, id: &'a str, expected: u64, pem: &'a str) -> SaveGithub<'a> {
    SaveGithub {
        key,
        id,
        expected,
        name: "GitHub engineering",
        app_id: "123",
        private_key_pem: pem,
    }
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with the runtime-role URL"]
async fn connection_retries_rotation_fencing_and_protected_credentials() -> TestResult {
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
    let first = create_and_replay(&store, &runtime_url).await?;
    let ready = store.finish_github_validation(&first.id, "").await?;
    assert_eq!(ready.connection.status, "ready");
    let active = ready.connection.active_credential_revision.clone();
    let failed = store
        .save_github_connection(candidate(
            &key(),
            &first.connection_id,
            1,
            "bad-replacement-key",
        ))
        .await?;
    let failed = store
        .finish_github_validation(&failed.id, "github_authentication_failed")
        .await?;
    assert_eq!(failed.state, "failed");
    assert_eq!(failed.connection.status, "ready");
    assert_eq!(failed.connection.active_credential_revision, active);
    let blocked = store
        .save_github_connection(candidate(
            &key(),
            &first.connection_id,
            2,
            "retry-after-outage-key",
        ))
        .await?;
    let unavailable = store
        .finish_github_validation(&blocked.id, "github_unavailable")
        .await?;
    assert_eq!(unavailable.state, "accepted");
    assert!(
        !unavailable
            .connection
            .candidate_credential_revision
            .is_empty()
    );
    let disable_key = key();
    let disabled = store
        .set_connection_enabled(&disable_key, &first.connection_id, 3, false)
        .await?;
    assert_eq!(disabled.connection.status, "disabled");
    let late = store.finish_github_validation(&blocked.id, "").await?;
    assert_eq!(late.state, "superseded");
    assert_eq!(late.connection.active_credential_revision, active);
    assert_eq!(late.connection.status, "disabled");
    assert_eq!(
        store
            .set_connection_enabled(&disable_key, &first.connection_id, 3, false)
            .await?
            .id,
        disabled.id
    );
    assert!(matches!(
        store
            .set_connection_enabled(&disable_key, &first.connection_id, 3, true)
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    let enabled = store
        .set_connection_enabled(&key(), &first.connection_id, 4, true)
        .await?;
    assert_eq!(enabled.connection.status, "ready");
    let a_key = key();
    let b_key = key();
    let (a, b) = tokio::join!(
        store.save_github_connection(candidate(&a_key, &first.connection_id, 5, "candidate-a")),
        store.save_github_connection(candidate(&b_key, &first.connection_id, 5, "candidate-b")),
    );
    assert_ne!(
        a.is_ok(),
        b.is_ok(),
        "both concurrent stale writers were accepted"
    );
    assert_safe_storage(&store, &runtime_url, &first.connection_id).await?;
    Ok(())
}

async fn assert_safe_storage(store: &WorkflowStore, runtime_url: &str, id: &str) -> TestResult {
    let listed = store.list_connections("", 100).await?;
    let history = store.list_connection_operations(id, "", 100).await?;
    let safe = serde_json::to_string(&(listed, history))?;
    for secret in [
        "sensitive-first-key",
        "bad-replacement-key",
        "retry-after-outage-key",
        "candidate-a",
        "candidate-b",
    ] {
        assert!(!safe.contains(secret));
    }
    let (client, driver) = ayni_orchestrator_workflow_postgres::connect(runtime_url).await?;
    let row=client.query_one("SELECT string_agg(safe_request || result,'') FROM workflow_connection_operations WHERE connection_id=$1", &[&id]).await?;
    let receipt: String = row.get(0);
    assert!(!receipt.contains("sensitive-first-key"));
    assert!(
        client
            .execute(
                "UPDATE workflow_connection_credentials SET value='changed'",
                &[]
            )
            .await
            .is_err()
    );
    assert!(
        client
            .execute("DELETE FROM workflow_connection_credentials", &[])
            .await
            .is_err()
    );
    driver.abort();
    assert!(matches!(
        store.list_connections("", 101).await,
        Err(StoreError::InvalidInput(_))
    ));
    Ok(())
}

async fn create_and_replay(
    store: &WorkflowStore,
    runtime_url: &str,
) -> Result<
    ayni_orchestrator_workflow_postgres::store::connections::Operation,
    Box<dyn std::error::Error + Send + Sync>,
> {
    let request = key();
    let first = store
        .save_github_connection(candidate(&request, "", 0, "sensitive-first-key"))
        .await?;
    assert_eq!(first.state, "accepted");
    let recovered = WorkflowStore::new(runtime_url)
        .get_connection_operation(&request)
        .await?;
    assert_eq!(first.id, recovered.id);
    let replay = store
        .save_github_connection(candidate(&request, "", 0, "sensitive-first-key"))
        .await?;
    assert_eq!(first.id, replay.id);
    assert!(matches!(
        store
            .save_github_connection(candidate(&request, "", 0, "changed-key"))
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        store
            .save_github_connection(candidate(&key(), &first.connection_id, 0, "key"))
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    Ok(first)
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with the runtime-role URL"]
#[allow(
    clippy::too_many_lines,
    reason = "the scenario retains credential rotation and admission ordering in one fixture"
)]
async fn provider_configuration_preserves_credentials_and_freezes_workspace_admission() -> TestResult
{
    use ayni_orchestrator_workflow_postgres::store::connections::{ProviderSettings, SaveProvider};
    let url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    ayni_orchestrator_workflow_postgres::migrate_for_runtime(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let store = WorkflowStore::new(url.clone());
    let save_key = key();
    let input = || SaveProvider {
        key: &save_key,
        id: "",
        expected: 0,
        name: "OpenAI development",
        provider: "openai",
        capability: "llm",
        settings: ProviderSettings::default(),
        credential: "fixture-openai-initial",
    };
    let candidate = store.save_provider_connection(input()).await?;
    assert_eq!(
        store.save_provider_connection(input()).await?.id,
        candidate.id
    );
    let mut conflict = input();
    conflict.credential = "fixture-conflicting-value";
    assert!(matches!(
        store.save_provider_connection(conflict).await,
        Err(StoreError::IdempotencyConflict)
    ));
    let ready = store
        .finish_provider_validation(&candidate.id, "", &["fixture-model".into()])
        .await?;
    let active = ready.connection.active_credential_revision.clone();
    let workspace = store
        .create_workspace(&key(), "Provider admission")
        .await?
        .resource;
    let project = store
        .create_project(
            &key(),
            &workspace.id,
            "Provider project",
            "Freeze explicit credentials",
        )
        .await?
        .resource;
    let conversation = store
        .create_conversation(&key(), &workspace.id, &project.id, "Provider conversation")
        .await?
        .resource;
    let initial = store.get_agent_configuration(&workspace.id, "").await?;
    let mut proposed = initial.clone();
    proposed.revision_id.clear();
    proposed.connection_id = candidate.connection_id.clone();
    proposed.provider = "openai".into();
    proposed.model = "fixture-model".into();
    let update = store
        .update_agent_configuration(&key(), &initial.revision_id, &proposed)
        .await?;
    let first = store
        .send_message(
            &key(),
            &workspace.id,
            &project.id,
            &conversation.id,
            "First binding",
        )
        .await?;
    let rotated = store
        .save_provider_connection(SaveProvider {
            key: &key(),
            id: &candidate.connection_id,
            expected: 1,
            name: "OpenAI development",
            provider: "openai",
            capability: "llm",
            settings: ProviderSettings::default(),
            credential: "fixture-openai-rotation",
        })
        .await?;
    let rotated = store
        .finish_provider_validation(&rotated.id, "", &["fixture-model".into()])
        .await?;
    let next = store
        .send_message(
            &key(),
            &workspace.id,
            &project.id,
            &conversation.id,
            "Second binding",
        )
        .await?;
    let (client, driver) = ayni_orchestrator_workflow_postgres::connect(&url).await?;
    for (turn, expected) in [
        (&first.turn_id, &active),
        (
            &next.turn_id,
            &rotated.connection.active_credential_revision,
        ),
    ] {
        let row=client.query_one("SELECT credential_revision,configuration_revision_id FROM workflow_turn_inputs WHERE turn_id=$1",&[&turn]).await?;
        assert_eq!(row.get::<_, String>(0), *expected);
        assert_eq!(row.get::<_, String>(1), update.revision_id);
    }
    driver.abort();
    let invalid = store
        .save_provider_connection(SaveProvider {
            key: &key(),
            id: &candidate.connection_id,
            expected: 2,
            name: "OpenAI development",
            provider: "openai",
            capability: "llm",
            settings: ProviderSettings::default(),
            credential: "fixture-invalid-rotation",
        })
        .await?;
    let failed = store
        .finish_provider_validation(&invalid.id, "provider_authentication_failed", &[])
        .await?;
    assert_eq!(
        failed.connection.active_credential_revision,
        rotated.connection.active_credential_revision
    );
    assert_eq!(failed.connection.models, vec!["fixture-model"]);
    store
        .set_connection_enabled(&key(), &candidate.connection_id, 3, false)
        .await?;
    assert!(matches!(
        store
            .send_message(
                &key(),
                &workspace.id,
                &project.id,
                &conversation.id,
                "Disabled connection"
            )
            .await,
        Err(StoreError::InvalidInput("connection_unavailable"))
    ));
    let receipts = serde_json::to_string(
        &store
            .list_connection_operations(&candidate.connection_id, "", 100)
            .await?,
    )?;
    for secret in [
        "fixture-openai-initial",
        "fixture-openai-rotation",
        "fixture-invalid-rotation",
    ] {
        assert!(!receipts.contains(secret));
    }
    let registry_key = key();
    let credentials = serde_json::to_string(&["fixture-push-pat", "fixture-pull-pat"])?;
    let registry = store
        .save_provider_connection(SaveProvider {
            key: &registry_key,
            id: "",
            expected: 0,
            name: "GHCR development",
            provider: "github",
            capability: "container_registry",
            settings: ProviderSettings {
                registry_destination: "ghcr.io/fixture/runtime".into(),
                push_username: "fixture-push".into(),
                pull_username: "fixture-pull".into(),
            },
            credential: &credentials,
        })
        .await?;
    assert_eq!(
        store.provider_candidate(&registry.id).await?.credential,
        credentials
    );
    assert!(store.github_candidate(&registry.id).await.is_err());
    assert!(
        store
            .provider_candidate(&registry.id)
            .await?
            .operation
            .connection
            .models
            .is_empty()
    );
    assert!(!serde_json::to_string(&registry)?.contains("fixture-push-pat"));
    Ok(())
}
