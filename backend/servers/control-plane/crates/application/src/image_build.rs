//! Artifact-producing steps share one engine and publication contract.
use serde::{Deserialize, Serialize};

/// The authority and source required by the artifact-producing step.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ImageBuildKind {
    RepositoryRuntime,
}

pub use ayni_orchestrator_runtime::image_build::ImagePublicationRequest;

use crate::execution::{
    ExecutionBackend, ExecutionResult, ExecutionState, ImageDiscovery, WorkloadRuntime,
};

/// One observation contract for all durable image producers.
pub enum BuildObservation {
    Missing,
    Running,
    Completed(String),
    Failed(String),
    EngineLost,
}

/// Observe candidate creation and engine continuity before advancing publication.
/// # Errors
/// Reports infrastructure failures without interpreting artifact-specific evidence.
pub async fn observe_build<E: ExecutionBackend>(
    backend: &E,
    name: &str,
    runtime: &WorkloadRuntime,
    started: bool,
) -> ExecutionResult<BuildObservation> {
    if started && !backend.engine_alive(name, runtime).await? {
        return Ok(BuildObservation::EngineLost);
    }
    Ok(match backend.coding_state(name).await? {
        ExecutionState::Missing => BuildObservation::Missing,
        ExecutionState::Running => BuildObservation::Running,
        ExecutionState::Succeeded => BuildObservation::Completed(backend.build_result(name).await?),
        ExecutionState::Failed => {
            BuildObservation::Failed(backend.operation_failure(name, "build").await?)
        }
    })
}

/// Registry readback must identify the exact candidate, including uncertain pushes.
/// # Errors
/// Rejects unrelated mutable-tag results unless a rebuild is authorized.
pub fn publication_readback(
    discovery: ImageDiscovery,
    candidate: Option<&str>,
    overwrite: bool,
) -> Result<ImageDiscovery, &'static str> {
    if let ImageDiscovery::Resolved { image_id, .. } = &discovery
        && candidate.is_some_and(|expected| expected != image_id)
    {
        return if overwrite {
            Ok(ImageDiscovery::Missing)
        } else {
            Err("runtime_tag_drift")
        };
    }
    Ok(discovery)
}
