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
