use ayni_orchestrator_worker::{Configuration, run};
use std::process::ExitCode;
use tracing::error;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let result = Configuration::from_environment().and_then(|configuration| run(&configuration));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(%error, "managed check worker failed");
            ExitCode::FAILURE
        }
    }
}
