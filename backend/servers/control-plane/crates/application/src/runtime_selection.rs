//! Select a new immutable runtime revision when its source or admitted policy identity changes.
use crate::identity::GitRevision;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionIdentity {
    pub policy_binding: String,
    pub image_repository: String,
    pub source_digest: Option<String>,
}

pub struct SelectRuntimeRevision {
    pub profile_id: String,
    pub source_revision: GitRevision,
    pub identity: SelectionIdentity,
    pub policy_version: String,
}

pub struct SelectedRevision {
    pub profile_version: u64,
    pub revision_id: String,
    pub source_revision: String,
    pub identity: Option<SelectionIdentity>,
}

#[derive(Debug, Error)]
pub enum SelectionError<E> {
    #[error("runtime revision persistence failed")]
    Storage(#[source] E),
    #[error("runtime revision policy is unsupported")]
    UnsupportedProfile,
}

pub trait Repository {
    type Error: std::error::Error;
    /// Load the currently selected revision, not a matching historical revision.
    /// # Errors
    /// Rejects unavailable, missing or corrupt state.
    fn selected(&self, profile_id: &str) -> Result<SelectedRevision, Self::Error>;
    /// Record/select the revision and profile event atomically, checking the observed version.
    /// Historical matching revisions must not replace a newly selected revision on policy revert.
    /// # Errors
    /// Rejects stale selection and persistence failure.
    fn commit_selection(
        &mut self,
        command: &SelectRuntimeRevision,
        previous: &SelectedRevision,
    ) -> Result<String, Self::Error>;
}

/// Identical observation replays the current selection; source or policy changes select a new one.
/// # Errors
/// Returns unsupported policy or storage failure.
pub fn select<R: Repository>(
    repository: &mut R,
    command: &SelectRuntimeRevision,
) -> Result<String, SelectionError<R::Error>> {
    let previous = repository
        .selected(&command.profile_id)
        .map_err(SelectionError::Storage)?;
    let identity = previous
        .identity
        .as_ref()
        .ok_or(SelectionError::UnsupportedProfile)?;
    if previous.source_revision == command.source_revision.as_str() && identity == &command.identity
    {
        return Ok(previous.revision_id);
    }
    repository
        .commit_selection(command, &previous)
        .map_err(SelectionError::Storage)
}

#[cfg(test)]
mod tests;
