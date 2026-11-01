use super::{Expected, Failure, Inputs, WORKSPACE};
use ayni_core::{CompletionScope, CompletionState, ExecutionMode, RunArtifact, TargetIdentity};
pub(super) fn validate_artifact(
    artifact: &RunArtifact,
    expected: &Expected,
    inputs: &Inputs,
    targets: &[TargetIdentity],
) -> Result<(), Failure> {
    let metadata = &artifact.metadata;
    let complete = &artifact.completion;
    let valid = metadata.execution_mode == ExecutionMode::Managed
        && metadata.environment_lock_fingerprint.as_deref() == Some(expected.lock.as_str())
        && metadata.ayni_version == inputs.lock.ayni_version()
        && metadata.contract_digest == inputs.policy_digest
        && metadata.repository_root == WORKSPACE
        && metadata.invocation.command == "check"
        && !metadata.tool_versions.is_empty()
        && complete.scope == CompletionScope::Repository
        && complete.state == CompletionState::Complete
        && complete.expected_targets == targets.len() as u64
        && complete.completed_targets == targets.len() as u64
        && complete.detected_targets == targets.len() as u64
        && complete.skipped_targets == 0
        && complete.issues.is_empty()
        && !targets.is_empty();
    if !valid
        || artifact
            .rows
            .iter()
            .any(|row| row.result.command_failure().is_some())
    {
        return Err(Failure::CompatibilityFailed);
    }
    let prebuilt = metadata
        .prebuilt_runtime
        .as_ref()
        .ok_or(Failure::CompatibilityFailed)?;
    if prebuilt.metadata_path != "/etc/ayni/runtime.json"
        || prebuilt.metadata_digest != expected.certificate
        || prebuilt.certificate != inputs.certificate
    {
        return Err(Failure::CompatibilityFailed);
    }
    validate_scope(artifact, targets)
}
fn validate_scope(artifact: &RunArtifact, targets: &[TargetIdentity]) -> Result<(), Failure> {
    let scope = artifact
        .metadata
        .invocation
        .scope
        .as_ref()
        .ok_or(Failure::CompatibilityFailed)?;
    if scope.workspace_root != WORKSPACE
        || scope.file.is_some()
        || scope.package.is_some()
        || !targets.iter().any(|target| {
            scope.path.as_deref() == (target.root != ".").then_some(target.root.as_str())
        })
    {
        return Err(Failure::CompatibilityFailed);
    }
    let languages: std::collections::BTreeSet<_> =
        targets.iter().map(|target| target.language).collect();
    if artifact
        .metadata
        .invocation
        .languages
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        != languages
    {
        return Err(Failure::CompatibilityFailed);
    }
    for target in targets {
        if !artifact.rows.iter().any(|row| {
            row.language == target.language
                && row.scope.workspace_root == WORKSPACE
                && row.scope.path.as_deref() == (target.root != ".").then_some(target.root.as_str())
                && row.scope.file.is_none()
                && row.scope.package.is_none()
        }) {
            return Err(Failure::CompatibilityFailed);
        }
    }
    Ok(())
}
