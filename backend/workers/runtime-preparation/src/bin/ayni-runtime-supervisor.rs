use ayni_orchestrator_runtime_preparation::supervisor::Supervisor;
#[tokio::main]
async fn main() {
    let Ok(supervisor) = Supervisor::configured().await else {
        eprintln!("runtime supervisor configuration unavailable");
        std::process::exit(2);
    };
    loop {
        if Box::pin(supervisor.tick()).await.is_err() {
            eprintln!("runtime preparation reconciliation requires recovery");
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
