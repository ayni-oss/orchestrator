//! Execution operations do not interpret runtime-profile evidence.
pub use crate::execution_settings::WorkloadRuntime;
use ayni_orchestrator_runtime::{coding::WorkerSpec, runtime_execution::ExecutionRequest};
use serde::Serialize;
use std::future::Future;

pub type ExecutionResult<T> = Result<T, ExecutionError>;

pub use error::{ExecutionError, ExecutionErrorKind};
mod error;

/// Application execution boundary. Profile inputs are opaque serialized data.
///
/// Every capability must be implemented explicitly: unsupported operations and
/// liveness must never be silently assumed by an incomplete adapter.
pub trait ExecutionBackend: Send + Sync + 'static {
    fn start_image_build<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<P>,
        token: &str,
        kind: &crate::image_build::ImageBuildKind,
    ) -> impl Future<Output = ExecutionResult<()>> + Send;

    fn engine_alive(
        &self,
        _session: &str,
        _runtime: &WorkloadRuntime,
    ) -> impl Future<Output = ExecutionResult<bool>> + Send;
    fn start_discovery(
        &self,
        _name: &str,
        _runtime: &WorkloadRuntime,
    ) -> impl Future<Output = ExecutionResult<()>> + Send;
    fn discover(&self, _name: &str)
    -> impl Future<Output = ExecutionResult<ImageDiscovery>> + Send;
    fn operation_failure(
        &self,
        _name: &str,
        _container: &str,
    ) -> impl Future<Output = ExecutionResult<String>> + Send;
    fn build_result(&self, _name: &str) -> impl Future<Output = ExecutionResult<String>> + Send;
    fn start_publish<P: Serialize + Sync>(
        &self,
        _name: &str,
        _runtime: &WorkloadRuntime,
        _request: &ExecutionRequest<P>,
        _image_id: &str,
        _overwrite_authorized: bool,
    ) -> impl Future<Output = ExecutionResult<()>> + Send;
    fn start<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<P>,
        token: &str,
        reporting: Option<&ayni_orchestrator_runtime::verification::ReportingAdmission>,
        event_authoritative: bool,
    ) -> impl Future<Output = ExecutionResult<()>> + Send;
    fn start_trust_lock<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<P>,
        token: &str,
        key_id: &str,
        public_key: &str,
    ) -> impl Future<Output = ExecutionResult<()>> + Send;
    fn trust_lock_result(&self, name: &str)
    -> impl Future<Output = ExecutionResult<String>> + Send;
    fn start_coding<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &WorkerSpec<P>,
        token: &str,
        image: &str,
        deadline: u32,
    ) -> impl Future<Output = ExecutionResult<()>> + Send;
    fn refresh_credentials(
        &self,
        name: &str,
        token: &str,
    ) -> impl Future<Output = ExecutionResult<()>> + Send;
    fn observe(&self, name: &str) -> impl Future<Output = ExecutionResult<Observation>> + Send;
    fn coding_state(
        &self,
        name: &str,
    ) -> impl Future<Output = ExecutionResult<ExecutionState>> + Send;
    fn coding_reporting_identity(
        &self,
        _name: &str,
    ) -> impl Future<
        Output = ExecutionResult<Option<ayni_orchestrator_runtime::coding_events::CodingAdmission>>,
    > + Send {
        async { Ok(None) }
    }
    fn cleanup(&self, name: &str) -> impl Future<Output = ExecutionResult<()>> + Send;
    fn coding_stopped(&self, name: &str) -> impl Future<Output = ExecutionResult<bool>> + Send;
}

pub use ayni_orchestrator_runtime::image_discovery::ImageDiscovery;

/// Resource completion is an observation, not an application assessment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionState {
    Missing,
    Running,
    Succeeded,
    Failed,
}

pub enum Observation {
    Missing,
    Running,
    Completed { digest: String, report: String },
    Failed(&'static str),
}
