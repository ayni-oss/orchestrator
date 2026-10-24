use ayni_orchestrator_control_plane::Configuration;
use std::{io, process::ExitCode};
use tracing::{Instrument as _, error, info};

fn main() -> ExitCode {
    let telemetry = ayni_orchestrator_observability::init("workflow-migrate");
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)
        .and_then(|runtime| {
            runtime.block_on(migrate().instrument(tracing::info_span!("workflow_migration")))
        });
    telemetry.shutdown();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(
                target: "ayni_orchestrator_control_plane::migration",
                reason_code = "workflow_migration_failed",
                error = %ayni_orchestrator_observability::redact(&error.to_string()),
                "workflow migration runner failed"
            );
            ExitCode::FAILURE
        }
    }
}

async fn migrate() -> Result<(), io::Error> {
    let configuration = Configuration::from_environment().map_err(io::Error::other)?;
    let database_url = configuration.migration_database_url()?;
    ayni_orchestrator_workflow_postgres::migrate_for_runtime(
        &database_url,
        configuration.runtime_database_role(),
    )
    .await
    .map_err(io::Error::other)?;
    info!(target: "ayni_orchestrator_control_plane::migration", "workflow schema migration and validation passed");
    Ok(())
}
