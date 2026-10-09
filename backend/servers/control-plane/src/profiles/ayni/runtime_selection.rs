//! Managed source observations become neutral immutable revision selection inputs.
use ayni_orchestrator_application::{
    identity::{GitRevision, InvalidGitRevision},
    runtime_selection::{SelectRuntimeRevision, SelectionIdentity},
};

pub fn command(
    profile_id: &str,
    revision: &str,
    fingerprint: &str,
    repository: &str,
    source_digest: &str,
    ayni_version: &str,
) -> Result<SelectRuntimeRevision, InvalidGitRevision> {
    Ok(SelectRuntimeRevision {
        profile_id: profile_id.into(),
        source_revision: GitRevision::parse(revision)?,
        identity: SelectionIdentity {
            policy_binding: fingerprint.into(),
            image_repository: repository.into(),
            source_digest: Some(source_digest.into()),
        },
        policy_version: ayni_version.into(),
    })
}
