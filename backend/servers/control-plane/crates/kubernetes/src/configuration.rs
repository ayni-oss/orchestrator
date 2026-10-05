//! Execution resource settings supplied by the application boundary.
//!
//! Scheduling, profile selection, repository discovery, and publication policy
//! belong to the control plane. Credentials here are Kubernetes Secret names.

pub use ayni_orchestrator_application::execution_settings::{
    BuildRuntime, WorkloadRuntime, image_repository,
};
use ayni_orchestrator_runtime::runtime_contract::RuntimeImage;

/// Deployment resources required by the Kubernetes execution adapter.
#[derive(Clone)]
pub struct ExecutionConfiguration {
    pub namespace: String,
    pub worker_image: String,
    pub output_claim: String,
    pub model_secret: String,
    /// Local Docker Desktop only; production keeps platform node selection.
    pub allow_emulation: bool,
}

impl ExecutionConfiguration {
    /// Validates deployment resource names and the trusted worker image identity.
    ///
    /// # Errors
    /// Returns a bounded error code for malformed names or an unpinned worker image.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !name(&self.namespace)
            || !name(&self.output_claim)
            || !name(&self.model_secret)
            || RuntimeImage::parse(&self.worker_image).is_err()
        {
            return Err("runtime_configuration_invalid");
        }
        Ok(())
    }
}

fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
