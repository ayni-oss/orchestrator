//! Immutable assessment admission, independent of evidence formats and provider protocols.
use thiserror::Error;

pub struct ImportAssessment {
    pub id: String,
    pub manifest_digest: String,
    pub repository_id: String,
    pub change_id: String,
    pub profile_revision_id: String,
    pub head_revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Receipt {
    pub id: String,
    pub head_revision: String,
}

pub struct RetainedAssessment {
    pub receipt: Receipt,
    pub manifest_digest: String,
}

pub struct Facts {
    pub profile_repository_id: String,
}

#[derive(Debug, Error)]
pub enum ImportError<E> {
    #[error("assessment persistence failed")]
    Storage(#[source] E),
    #[error("assessment identity was reused with different evidence")]
    IdentityConflict,
    #[error("assessment profile does not belong to the review repository")]
    ParentMismatch,
}

/// Evidence has already passed profile admission; persistence retains immutable bytes and events.
pub trait Repository {
    type Error: std::error::Error;
    /// Load retained immutable evidence identity.
    ///
    /// # Errors
    /// Returns an error for unavailable or corrupt retained state.
    fn replay(&self, id: &str) -> Result<Option<RetainedAssessment>, Self::Error>;
    /// Load repository membership for the referenced immutable profile revision.
    ///
    /// # Errors
    /// Returns an error for missing or unreadable durable resources.
    fn facts(&self, command: &ImportAssessment) -> Result<Facts, Self::Error>;
    /// Retain evidence before committing assessment and evidence-reference events together.
    ///
    /// # Errors
    /// Returns an error if evidence retention or the atomic transaction fails.
    fn commit(&mut self, command: &ImportAssessment) -> Result<Receipt, Self::Error>;
}

/// Import immutable admitted evidence, replaying it before evaluating current profile facts.
/// Historical heads remain valid history; readiness separately determines current authority.
///
/// # Errors
/// Rejects conflicting evidence, mismatched repository membership and persistence failure.
pub fn import<R: Repository>(
    repository: &mut R,
    command: &ImportAssessment,
) -> Result<Receipt, ImportError<R::Error>> {
    if let Some(retained) = repository
        .replay(&command.id)
        .map_err(ImportError::Storage)?
    {
        if retained.manifest_digest != command.manifest_digest {
            return Err(ImportError::IdentityConflict);
        }
        return Ok(retained.receipt);
    }
    let facts = repository.facts(command).map_err(ImportError::Storage)?;
    if facts.profile_repository_id != command.repository_id {
        return Err(ImportError::ParentMismatch);
    }
    repository.commit(command).map_err(ImportError::Storage)
}

#[cfg(test)]
mod tests;
