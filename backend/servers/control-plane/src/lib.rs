//! Native gRPC, operational HTTP, and durable orchestration for Ayni Orchestrator.
mod adapters;
mod coordinators;
mod process;
mod profiles;
pub mod registry_probe;

pub use adapters::grpc::access::{AccessConfigurationError, CustomClientAccess};
pub use adapters::grpc::{serve as serve_grpc, serve_custom as serve_custom_grpc};
pub use adapters::http::router;
pub use adapters::models::{AvailableModel, ModelCatalog, ModelCatalogError};
pub use ayni_orchestrator_application::execution::ExecutionBackend;
pub use coordinators::reconciliation::RuntimeReconciler;
pub use coordinators::{AppState, ReconciliationError};
pub use process::configuration::{Configuration, ConfigurationError, CustomGrpcConfiguration};

pub(crate) fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_owned()
}

pub use process::supervision::BackgroundTasks;
