//! Dedicated repository-scoped publisher. Repository code is never executed.
#[path = "../publication/config.rs"]
mod config;
#[path = "../publication/git.rs"]
mod git;
#[path = "../publication/github.rs"]
mod github;
#[path = "../publication/workflow.rs"]
mod workflow;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() -> Result {
    let telemetry = ayni_orchestrator_observability::init("task-publisher");
    let delivery = ayni_orchestrator_execution::transport::Delivery::from_environment(
        "ORCHESTRATOR_PUBLICATION_AUTHORITY_TOKEN_PATH",
    )
    .unwrap_or_else(|_| {
        eprintln!("publisher telemetry unavailable; capture incomplete");
        None
    });
    let result = run().await;
    if let Some(delivery) = delivery {
        delivery.finish().await;
    }
    telemetry.shutdown();
    result
}
async fn run() -> Result {
    let configuration = config::Configuration::load()?;
    let http = github::client()?;
    loop {
        let operation = workflow::poll(&http, &configuration);
        tokio::select! {
            result = operation => if result.is_err() { eprintln!("task publication awaits reconciliation; inspect retained publication state"); },
            () = ayni_orchestrator_execution::process::termination() => return Ok(())
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
