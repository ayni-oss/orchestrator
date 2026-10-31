//! Catalog/project browser acceptance against real `PostgreSQL`, gRPC and gateway.
use ayni_orchestrator_control_plane::{
    gateway::{Gateway, router},
    serve_grpc,
};
use ayni_orchestrator_workflow_postgres::{
    WorkflowStore,
    store::{
        connections::SaveGithub,
        repositories::{Connect, ObservedRepository},
    },
};
use std::{env, error::Error, net::TcpListener};
use tokio::sync::watch;
#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL and Playwright Chromium"]
async fn repository_browser_preserves_catalog_and_project_receipts() -> Result<(), Box<dyn Error>> {
    let url = env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    let store = WorkflowStore::new(url.clone());
    let key = || uuid::Uuid::now_v7().to_string();
    let source = store
        .save_github_connection(SaveGithub {
            key: &key(),
            id: "",
            expected: 0,
            name: "Repository browser fixture",
            app_id: "123",
            private_key_pem: "synthetic-repository-browser-key",
        })
        .await?;
    store.finish_github_validation(&source.id, "").await?;
    let command = key();
    let provider_id = u64::try_from(uuid::Uuid::new_v4().as_u128() % 1_000_000_000_000)? + 1;
    store
        .begin_repository_connect(Connect {
            key: &command,
            connection_id: &source.connection_id,
            expected_revision: 1,
            installation_id: 123,
            provider_repository_id: provider_id,
        })
        .await?;
    let repo = store
        .finish_repository_connect(
            &command,
            Ok(ObservedRepository {
                provider_repository_id: provider_id,
                full_name: "fixture/browser-repository".into(),
                default_branch: "main".into(),
                private: true,
            }),
        )
        .await?
        .repository
        .ok_or("fixture repository missing")?;
    seed_static_inspection(&store, &repo.id).await?;
    let a = store
        .create_workspace(&key(), "Repository browser A")
        .await?
        .resource;
    let b = store
        .create_workspace(&key(), "Repository browser B")
        .await?
        .resource;
    let pa = store
        .create_project(&key(), &a.id, "Source A", "Use a shared catalog")
        .await?
        .resource;
    let pb = store
        .create_project(&key(), &b.id, "Source B", "Reuse a shared catalog")
        .await?
        .resource;
    let address = TcpListener::bind("127.0.0.1:0")?.local_addr()?;
    let (shutdown, rx) = watch::channel(false);
    let grpc = tokio::spawn(serve_grpc(address, WorkflowStore::new(url), rx));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let gateway_address = listener.local_addr()?;
    let app = router(Gateway::new(gateway_address, address)?);
    let gateway = tokio::spawn(async move { axum::serve(listener, app).await });
    let result = tokio::process::Command::new("node")
        .args(["--test", "apps/web/tests/repositories.browser.mjs"])
        .current_dir(env!("CARGO_MANIFEST_DIR").to_owned() + "/../../..")
        .env(
            "AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT",
            gateway_address.port().to_string(),
        )
        .env("AYNI_REPOSITORY_FIXTURE", repo.id)
        .env("AYNI_REPOSITORY_SOURCE", source.connection_id)
        .env("AYNI_REPOSITORY_WORKSPACE_A", a.id)
        .env("AYNI_REPOSITORY_WORKSPACE_B", b.id)
        .env("AYNI_REPOSITORY_PROJECT_A", pa.id)
        .env("AYNI_REPOSITORY_PROJECT_B", pb.id)
        .status()
        .await;
    gateway.abort();
    let _ = shutdown.send(true);
    grpc.await??;
    if !result?.success() {
        return Err("repository browser acceptance failed".into());
    }
    Ok(())
}

async fn seed_static_inspection(
    store: &WorkflowStore,
    repository_id: &str,
) -> Result<(), Box<dyn Error>> {
    let inspection_key = uuid::Uuid::now_v7().to_string();
    store
        .begin_repository_inspection(
            ayni_orchestrator_workflow_postgres::store::repository_inspections::Inspect {
                key: &inspection_key,
                repository_id,
                expected_connection_revision: 1,
                commit_sha: &"a".repeat(40),
            },
        )
        .await?;
    store.finish_repository_inspection(&inspection_key, Ok(
        ayni_orchestrator_workflow_postgres::store::repository_inspections::InspectionResult {
            policy_digest: "sha256:fixture-policy".into(), lock_digest: "sha256:fixture-lock".into(),
            lock_fingerprint: String::new(), ayni_version: String::new(), lock_schema: String::new(),
            tools: vec![], trust_key_ids: vec![], findings: vec![
                ayni_orchestrator_workflow_postgres::store::repository_inspections::Finding {
                    code: "policy_missing".into(), path: ".ayni.toml".into(),
                    action: "Commit an Ayni policy before preparing this repository.".into(),
                }
            ],
        }
    )).await?;
    Ok(())
}
