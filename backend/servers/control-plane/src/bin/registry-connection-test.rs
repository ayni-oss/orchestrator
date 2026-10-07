//! Isolated registry probe. Credentials are read only from Job-mounted files.
use ayni_orchestrator_control_plane::registry_probe::{RegistryTestReport, run};
use std::{env, fs, process::ExitCode};

#[tokio::main]
async fn main() -> ExitCode {
    let namespace = env::var("REGISTRY_NAMESPACE").unwrap_or_default();
    let operation = env::var("REGISTRY_OPERATION").unwrap_or_default();
    let cleanup = env::var("REGISTRY_CLEANUP_DIGEST").ok();
    let pull = fs::metadata("/credentials/pull/config.json")
        .is_ok()
        .then_some("/credentials/pull/config.json");
    let report = run(
        &namespace,
        &operation,
        "/credentials/push/config.json",
        pull,
        cleanup.as_deref(),
    )
    .await
    .unwrap_or_else(|reason| RegistryTestReport {
        failure_reason: reason.into(),
        ..RegistryTestReport::default()
    });
    let result = serde_json::to_vec(&report)
        .ok()
        .and_then(|bytes| fs::write("/dev/termination-log", bytes).ok());
    if result.is_some() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
