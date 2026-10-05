//! Kubernetes implementation of application execution operations.
use crate::execution::RuntimeKubernetes;
use ayni_orchestrator_application::execution::{
    ExecutionBackend, ExecutionError, ExecutionResult, ExecutionState, Observation, WorkloadRuntime,
};
use ayni_orchestrator_runtime::{coding::CodingRequest, runtime_execution::ExecutionRequest};
use serde::Serialize;

impl ExecutionBackend for RuntimeKubernetes {
    async fn start<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<P>,
        token: &str,
    ) -> ExecutionResult<()> {
        self.start(name, runtime, request, token)
            .await
            .map_err(ExecutionError::from)
    }
    async fn start_build<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<P>,
        token: &str,
    ) -> ExecutionResult<()> {
        self.start_build(name, runtime, request, token)
            .await
            .map_err(ExecutionError::from)
    }
    async fn start_coding<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &CodingRequest<P>,
        token: &str,
        image: &str,
        deadline: u32,
    ) -> ExecutionResult<()> {
        self.start_coding(name, runtime, request, token, image, deadline)
            .await
            .map_err(ExecutionError::from)
    }
    async fn observe(&self, name: &str) -> ExecutionResult<Observation> {
        self.observe(name).await.map_err(ExecutionError::from)
    }
    async fn coding_state(&self, name: &str) -> ExecutionResult<ExecutionState> {
        self.coding_state(name).await.map_err(ExecutionError::from)
    }
    async fn cleanup(&self, name: &str) -> ExecutionResult<()> {
        self.cleanup(name).await.map_err(ExecutionError::from)
    }
    async fn coding_stopped(&self, name: &str) -> ExecutionResult<bool> {
        self.coding_stopped(name)
            .await
            .map_err(ExecutionError::from)
    }
}
