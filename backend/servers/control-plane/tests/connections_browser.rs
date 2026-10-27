//! Combined real `PostgreSQL`, gRPC, gateway and product-browser acceptance.
use ayni_orchestrator_control_plane::{
    gateway::{Gateway, router},
    serve_grpc,
};
use ayni_orchestrator_workflow_postgres::WorkflowStore;
use std::{
    env,
    error::Error,
    net::{SocketAddr, TcpListener},
};
use tokio::sync::watch;

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL, pnpm and installed Playwright Chromium"]
async fn product_browser_uses_canonical_postgres_connections() -> Result<(), Box<dyn Error>> {
    let url = env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    // Deterministic model discovery fixture; no external OpenAI request or real key.
    let store = WorkflowStore::new(url.clone());
    let fixture = store
        .save_provider_connection(
            ayni_orchestrator_workflow_postgres::store::connections::SaveProvider {
                key: &uuid::Uuid::now_v7().to_string(),
                id: "",
                expected: 0,
                name: "Browser OpenAI fixture",
                provider: "openai",
                capability: "llm",
                settings: ayni_orchestrator_workflow_postgres::store::connections::ProviderSettings::default(),
                credential: "browser-only-synthetic-key",
            },
        )
        .await?;
    store
        .finish_provider_validation(&fixture.id, "", &["fixture-model".into()])
        .await?;
    let grpc_address = TcpListener::bind("127.0.0.1:0")?.local_addr()?;
    let (shutdown, receiver) = watch::channel(false);
    let grpc = tokio::spawn(serve_grpc(grpc_address, WorkflowStore::new(url), receiver));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address: SocketAddr = listener.local_addr()?;
    let app = router(Gateway::new(address, grpc_address)?);
    let gateway = tokio::spawn(async move { axum::serve(listener, app).await });
    let result = tokio::process::Command::new("node")
        .args(["--test", "apps/web/tests/connections.browser.mjs"])
        .current_dir(env!("CARGO_MANIFEST_DIR").to_owned() + "/../../..")
        .env(
            "AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT",
            address.port().to_string(),
        )
        .env("AYNI_BROWSER_CONNECTION_ID", fixture.connection_id)
        .status()
        .await;
    gateway.abort();
    let _ = shutdown.send(true);
    grpc.await??;
    if !result?.success() {
        return Err("product browser acceptance failed".into());
    }
    Ok(())
}
