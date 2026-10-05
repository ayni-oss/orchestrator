//! Persistence codec and atomic mutations for the application runtime activation port.
use super::{CleanStore, CleanStoreError, timestamp, unix_seconds, v1};
use ayni_orchestrator_application::runtime_activation::{
    ActivateRuntime, ActivationError, Mutation, ProfileFacts, Repository, RevisionFacts,
};

pub(crate) fn activation_error(error: ActivationError<CleanStoreError>) -> CleanStoreError {
    match error {
        ActivationError::Storage(error) => error,
        ActivationError::StaleObservation => CleanStoreError::StaleVersion,
        ActivationError::UnsupportedProfile => CleanStoreError::InvalidTransition,
        ActivationError::IdentityConflict => CleanStoreError::IdempotencyConflict,
    }
}

impl Repository for CleanStore {
    type Error = CleanStoreError;

    fn profile(&self, id: &str) -> Result<ProfileFacts, Self::Error> {
        let profile = self.runtime_profile(id)?;
        Ok(ProfileFacts {
            version: profile.resource_version,
            selected_revision_id: profile.active_revision_id,
            active_artifact_id: profile.active_runtime_artifact_id,
            ready: profile.state == v1::RuntimeProfileState::Ready as i32,
        })
    }

    fn revision(&self, id: &str) -> Result<RevisionFacts, Self::Error> {
        let revision = self.runtime_profile_revision(id)?;
        let verification_binding = match revision.configuration {
            Some(v1::runtime_profile_revision::Configuration::AyniManaged(configuration)) => {
                Some(format!("sha256:{}", configuration.lock_fingerprint))
            }
            _ => None,
        };
        Ok(RevisionFacts {
            source_revision: revision.repository_revision,
            verification_binding,
        })
    }

    fn artifact_digest(&self, id: &str) -> Result<String, Self::Error> {
        Ok(self.runtime_artifact(id)?.resolved_digest)
    }

    fn commit(
        &mut self,
        command: &ActivateRuntime,
        facts: &ProfileFacts,
        mutation: Mutation,
    ) -> Result<(), Self::Error> {
        let profile = self.runtime_profile(&command.profile_id)?;
        if profile.resource_version != facts.version
            || profile.active_revision_id != facts.selected_revision_id
            || profile.active_runtime_artifact_id != facts.active_artifact_id
        {
            return Err(CleanStoreError::StaleVersion);
        }
        if mutation == Mutation::RestoreReady {
            self.transition_runtime_profile(
                &command.profile_id,
                facts.version,
                v1::RuntimeProfileState::Ready,
                None,
            )?;
            return Ok(());
        }
        let revision = self.runtime_profile_revision(&command.revision_id)?;
        let Some(v1::runtime_profile_revision::Configuration::AyniManaged(configuration)) =
            revision.configuration
        else {
            return Err(CleanStoreError::InvalidTransition);
        };
        let artifact = &command.artifact;
        self.record_runtime_artifact(
            &command.profile_id,
            v1::RuntimeArtifact {
                runtime_profile_revision_id: command.revision_id.clone(),
                oci_repository: configuration.oci_repository,
                discovery_tag: artifact
                    .image
                    .rsplit_once(':')
                    .filter(|_| !artifact.image.contains('@'))
                    .map(|(_, tag)| tag.to_owned()),
                resolved_digest: artifact.digest.clone(),
                platform: Some(v1::OciPlatform {
                    operating_system: artifact.operating_system.clone(),
                    architecture: artifact.architecture.clone(),
                    variant: None,
                }),
                origin: Some(v1::RuntimeArtifactOrigin {
                    origin: Some(v1::runtime_artifact_origin::Origin::External(
                        v1::ExternalArtifactOrigin {
                            supplied_reference: artifact.image.clone(),
                            observed_at: Some(timestamp(unix_seconds()?)),
                        },
                    )),
                }),
                status: v1::RuntimeArtifactStatus::Verified as i32,
                verified_metadata: artifact
                    .metadata
                    .iter()
                    .map(|(key, value)| v1::VerifiedMetadata {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect(),
                ..Default::default()
            },
        )?;
        Ok(())
    }
}
