use ayni_orchestrator_runtime_preparation::supervisor::Supervisor;
#[tokio::main]
async fn main() {
    let _telemetry = ayni_orchestrator_observability::init("runtime-preparation-supervisor");
    let Ok(supervisor) = Supervisor::configured().await else {
        eprintln!("runtime supervisor configuration unavailable");
        std::process::exit(2);
    };
    loop {
        // The supervisor reports bounded, credential-free context and throttles repeated failures.
        let _ = Box::pin(supervisor.tick()).await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
