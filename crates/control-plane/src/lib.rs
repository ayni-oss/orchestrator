//! Native gRPC, operational HTTP, and durable orchestration for Ayni Orchestrator.

mod application;
mod application_metrics;
mod clean_store;
mod configuration;
pub(crate) mod github_setup;
mod grpc;
mod grpc_access;
mod http;
pub(crate) mod managed_environment;
mod model_catalog;
mod onboarding;
pub(crate) mod onboarding_templates;
mod runtime_configuration;
mod runtime_reconciler;
mod runtime_source;

pub(crate) use application::with_store;
pub use application::{AppState, ReconciliationError};
pub use configuration::{Configuration, ConfigurationError, CustomGrpcConfiguration};
pub use grpc::{serve as serve_grpc, serve_custom as serve_custom_grpc};
pub use grpc_access::{AccessConfigurationError, CustomClientAccess};
pub use http::router;
pub use model_catalog::{AvailableModel, ModelCatalog, ModelCatalogError};
pub use runtime_reconciler::RuntimeReconciler;

pub(crate) fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_owned()
}
