use ayni_orchestrator_worker::{Configuration, run};
use std::process::ExitCode;
use tracing::error;

fn main() -> ExitCode {
    ayni_orchestrator_observability::init("managed-check-worker");
    let process_span = ayni_orchestrator_observability::process_span("managed-check-worker");
    let _process = process_span.enter();

    let result = Configuration::from_environment().and_then(|configuration| run(&configuration));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(
                reason_code = "managed_check_failed",
                error = %ayni_orchestrator_observability::redact(&error.to_string()),
                "managed check worker failed"
            );
            ExitCode::FAILURE
        }
    }
}
