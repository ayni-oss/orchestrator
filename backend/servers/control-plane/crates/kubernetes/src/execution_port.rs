//! Kubernetes implementation of application execution operations.
use crate::execution::RuntimeKubernetes;
use ayni_orchestrator_application::execution::{
    ExecutionBackend, ExecutionError, ExecutionResult, ExecutionState, Observation, WorkloadRuntime,
};
use ayni_orchestrator_runtime::{coding::WorkerSpec, runtime_execution::ExecutionRequest};
use serde::Serialize;

impl ExecutionBackend for RuntimeKubernetes {
    async fn operation_failure(&self, name: &str, container: &str) -> ExecutionResult<String> {
        Ok(self
            .operation_message(name, container)
            .await
            .unwrap_or_else(|_| "runtime_publication_failed".into()))
    }
    async fn build_result(&self, name: &str) -> ExecutionResult<String> {
        self.build_result(name).await.map_err(ExecutionError::from)
    }
    async fn start_publish<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<P>,
        image_id: &str,
        overwrite_authorized: bool,
    ) -> ExecutionResult<()> {
        self.start_publish(name, runtime, request, image_id, overwrite_authorized)
            .await
            .map_err(ExecutionError::from)
    }
    async fn start_discovery(&self, name: &str, runtime: &WorkloadRuntime) -> ExecutionResult<()> {
        self.start_discovery(name, runtime)
            .await
            .map_err(ExecutionError::from)
    }
    async fn discover(
        &self,
        name: &str,
    ) -> ExecutionResult<ayni_orchestrator_application::execution::ImageDiscovery> {
        self.discover(name).await.map_err(ExecutionError::from)
    }
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
    async fn start_trust_lock<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<P>,
        token: &str,
        key_id: &str,
        public_key: &str,
    ) -> ExecutionResult<()> {
        self.start_trust_lock(name, runtime, request, token, key_id, public_key)
            .await
            .map_err(ExecutionError::from)
    }
    async fn trust_lock_result(&self, name: &str) -> ExecutionResult<String> {
        self.trust_lock_result(name)
            .await
            .map_err(ExecutionError::from)
    }
    async fn start_coding<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &WorkerSpec<P>,
        token: &str,
        image: &str,
        deadline: u32,
    ) -> ExecutionResult<()> {
        self.start_coding(name, runtime, request, token, image, deadline)
            .await
            .map_err(ExecutionError::from)
    }
    async fn refresh_credentials(&self, name: &str, token: &str) -> ExecutionResult<()> {
        self.refresh_credentials(name, token)
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
