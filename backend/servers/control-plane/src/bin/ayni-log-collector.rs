#[tokio::main]
async fn main() -> std::process::ExitCode {
    if ayni_orchestrator_control_plane::log_collector::run()
        .await
        .is_ok()
    {
        std::process::ExitCode::SUCCESS
    } else {
        eprintln!("log collector unavailable");
        std::process::ExitCode::FAILURE
    }
}
