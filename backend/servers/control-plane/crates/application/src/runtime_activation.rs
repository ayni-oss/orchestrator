//! Activation of an admitted runtime observation against the selected immutable revision.
use crate::identity::GitRevision;
use thiserror::Error;

pub struct ActivateRuntime {
    pub profile_id: String,
    pub revision_id: String,
    pub source_revision: GitRevision,
    pub verification_binding: String,
    pub artifact: VerifiedArtifact,
}

pub struct VerifiedArtifact {
    pub image: String,
    pub digest: String,
    pub operating_system: String,
    pub architecture: String,
    pub metadata: Vec<(String, String)>,
}

pub struct ProfileFacts {
    pub version: u64,
    pub selected_revision_id: Option<String>,
    pub active_artifact_id: Option<String>,
    pub ready: bool,
}

pub struct RevisionFacts {
    pub source_revision: String,
    /// The profile adapter supplies the evidence binding for supported policy.
    pub verification_binding: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mutation {
    RestoreReady,
    RecordArtifact,
}

#[derive(Debug, Error)]
pub enum ActivationError<E> {
    #[error("runtime activation persistence failed")]
    Storage(#[source] E),
    #[error("runtime observation no longer matches the selected revision")]
    StaleObservation,
    #[error("runtime activation policy is unsupported")]
    UnsupportedProfile,
    #[error("runtime revision is already bound to a different digest")]
    IdentityConflict,
}

pub trait Repository {
    type Error: std::error::Error;
    /// # Errors
    /// Returns an error for unavailable, missing or corrupt durable state.
    fn profile(&self, id: &str) -> Result<ProfileFacts, Self::Error>;
    /// # Errors
    /// Returns an error for unavailable, missing or corrupt immutable revision state.
    fn revision(&self, id: &str) -> Result<RevisionFacts, Self::Error>;
    /// # Errors
    /// Returns an error for unavailable, missing or corrupt retained artifact state.
    fn artifact_digest(&self, id: &str) -> Result<String, Self::Error>;
    /// Atomically persist the artifact/profile change and events after checking the observed version.
    /// # Errors
    /// Rejects stale profile facts and persistence failure without partial events.
    fn commit(
        &mut self,
        command: &ActivateRuntime,
        facts: &ProfileFacts,
        mutation: Mutation,
    ) -> Result<(), Self::Error>;
}

/// Profile policy has admitted the evidence before this lifecycle operation.
/// A late observation cannot select a revision or replace an immutable digest binding.
/// # Errors
/// Rejects stale source/evidence, unsupported policy, conflicting digest and storage failure.
pub fn activate<R: Repository>(
    repository: &mut R,
    command: &ActivateRuntime,
) -> Result<(), ActivationError<R::Error>> {
    let profile = repository
        .profile(&command.profile_id)
        .map_err(ActivationError::Storage)?;
    if profile.selected_revision_id.as_deref() != Some(command.revision_id.as_str()) {
        return Err(ActivationError::StaleObservation);
    }
    let revision = repository
        .revision(&command.revision_id)
        .map_err(ActivationError::Storage)?;
    if revision.source_revision != command.source_revision.as_str() {
        return Err(ActivationError::StaleObservation);
    }
    let binding = revision
        .verification_binding
        .ok_or(ActivationError::UnsupportedProfile)?;
    if binding != command.verification_binding {
        return Err(ActivationError::StaleObservation);
    }
    let mutation = if let Some(id) = &profile.active_artifact_id {
        let digest = repository
            .artifact_digest(id)
            .map_err(ActivationError::Storage)?;
        if digest != command.artifact.digest {
            return Err(ActivationError::IdentityConflict);
        }
        if profile.ready {
            return Ok(());
        }
        Mutation::RestoreReady
    } else {
        Mutation::RecordArtifact
    };
    repository
        .commit(command, &profile, mutation)
        .map_err(ActivationError::Storage)
}

#[cfg(test)]
mod tests;
