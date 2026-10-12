//! Native gRPC, operational HTTP, and durable orchestration for Ayni Orchestrator.
mod adapters;
mod coordinators;
mod process;
mod profiles;
pub mod registry_probe;

pub use adapters::grpc::access::{AccessConfigurationError, CustomClientAccess};
pub use adapters::grpc::{serve as serve_grpc, serve_custom as serve_custom_grpc};
pub use adapters::http::reporting::router as reporting_router;
pub use adapters::http::router;
pub use adapters::models::{AvailableModel, ModelCatalog, ModelCatalogError};
pub use ayni_orchestrator_application::execution::ExecutionBackend;
pub use coordinators::reconciliation::RuntimeReconciler;
pub use coordinators::{AppState, OutboxPublisher, ReconciliationError, VerificationConsumer};
pub use process::configuration::{Configuration, ConfigurationError, CustomGrpcConfiguration};

pub(crate) fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_owned()
}

pub use process::supervision::BackgroundTasks;

/// Internal shadow reporting configuration, independent from product transports.
/// # Errors
/// Returns a safe configuration error for malformed runtime settings.
pub fn reporting_configuration() -> Result<
    Option<(
        String,
        ayni_orchestrator_runtime::verification::ReportingConfiguration,
    )>,
    &'static str,
> {
    Ok(
        process::runtime_configuration::RuntimeConfiguration::from_environment()?.and_then(
            |runtime| {
                runtime
                    .reporting
                    .map(|reporting| (runtime.namespace, reporting))
            },
        ),
    )
}
