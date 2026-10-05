//! Map an admitted managed verification report into a neutral activation command.
use ayni_orchestrator_application::{
    identity::{GitRevision, InvalidGitRevision},
    runtime_activation::{ActivateRuntime, VerifiedArtifact},
};
use ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport;

pub fn command(
    profile_id: &str,
    revision_id: &str,
    image: &str,
    digest: &str,
    report: &VerificationReport,
) -> Result<ActivateRuntime, InvalidGitRevision> {
    Ok(ActivateRuntime {
        profile_id: profile_id.into(),
        revision_id: revision_id.into(),
        source_revision: GitRevision::parse(&report.revision)?,
        verification_binding: report.runtime.lock_fingerprint.clone(),
        artifact: VerifiedArtifact {
            image: image.into(),
            digest: digest.into(),
            operating_system: "linux".into(),
            architecture: report
                .runtime
                .platform
                .strip_prefix("linux/")
                .unwrap_or("")
                .into(),
            metadata: vec![
                (
                    "certificate-key-id".into(),
                    report.runtime.certificate_key_id.clone(),
                ),
                (
                    "runtime-metadata-digest".into(),
                    report.runtime.metadata_digest.clone(),
                ),
                (
                    "protected-content-root".into(),
                    report.runtime.protected_content_root.clone(),
                ),
                ("source-lock-digest".into(), report.lock_digest.clone()),
                (
                    "lock-fingerprint".into(),
                    report.runtime.lock_fingerprint.clone(),
                ),
            ],
        },
    })
}
