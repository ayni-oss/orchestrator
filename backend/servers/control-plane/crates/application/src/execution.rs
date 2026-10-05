//! Execution operations do not interpret runtime-profile evidence.
pub use crate::execution_settings::WorkloadRuntime;
use ayni_orchestrator_runtime::{coding::CodingRequest, runtime_execution::ExecutionRequest};
use serde::Serialize;
use std::future::Future;

pub type ExecutionResult<T> = Result<T, &'static str>;

/// Application execution boundary. Profile inputs are opaque serialized data.
pub trait ExecutionBackend: Send + Sync + 'static {
    fn start<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<P>,
        token: &str,
    ) -> impl Future<Output = ExecutionResult<()>> + Send;
    fn start_build<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<P>,
        token: &str,
    ) -> impl Future<Output = ExecutionResult<()>> + Send;
    fn start_coding<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &CodingRequest<P>,
        token: &str,
        image: &str,
        deadline: u32,
    ) -> impl Future<Output = ExecutionResult<()>> + Send;
    fn observe(&self, name: &str) -> impl Future<Output = ExecutionResult<Observation>> + Send;
    fn coding_state(
        &self,
        name: &str,
    ) -> impl Future<Output = ExecutionResult<ExecutionState>> + Send;
    fn cleanup(&self, name: &str) -> impl Future<Output = ExecutionResult<()>> + Send;
    fn coding_stopped(&self, name: &str) -> impl Future<Output = ExecutionResult<bool>> + Send;
}

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
