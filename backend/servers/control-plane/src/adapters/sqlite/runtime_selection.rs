//! Runtime selection persistence codec; lifecycle replay/change decisions belong to application.
use super::{CleanStore, CleanStoreError, v1};
use ayni_orchestrator_application::runtime_selection::{
    Repository, SelectRuntimeRevision, SelectedRevision, SelectionError, SelectionIdentity,
};

pub(crate) fn selection_error(error: SelectionError<CleanStoreError>) -> CleanStoreError {
    match error {
        SelectionError::Storage(error) => error,
        SelectionError::UnsupportedProfile => CleanStoreError::InvalidTransition,
    }
}
impl CleanStore {
    pub(crate) fn select_observed_runtime_revision(
        &mut self,
        command: &SelectRuntimeRevision,
        observed_revision: Option<&str>,
        observed_registry: &v1::RuntimeRegistrySelection,
    ) -> Result<String, CleanStoreError> {
        let profile = self.runtime_profile(&command.profile_id)?;
        if profile.active_revision_id.as_deref() != observed_revision
            || self.runtime_registry(&profile)? != *observed_registry
        {
            return Err(CleanStoreError::StaleVersion);
        }
        ayni_orchestrator_application::runtime_selection::select(self, command)
            .map_err(selection_error)
    }
}

impl Repository for CleanStore {
    type Error = CleanStoreError;
    fn selected(&self, profile_id: &str) -> Result<SelectedRevision, Self::Error> {
        let profile = self.runtime_profile(profile_id)?;
        let revision = self.runtime_profile_revision(
            profile
                .active_revision_id
                .as_deref()
                .ok_or(CleanStoreError::CorruptState)?,
        )?;
        let identity = match revision.configuration {
            Some(v1::runtime_profile_revision::Configuration::AyniManaged(configuration)) => {
                Some(SelectionIdentity {
                    policy_binding: configuration.lock_fingerprint,
                    image_repository: configuration.oci_repository,
                    source_digest: configuration.runtime_source_digest,
                })
            }
            _ => None,
        };
        Ok(SelectedRevision {
            profile_version: profile.resource_version,
            revision_id: revision.id,
            source_revision: revision.repository_revision,
            identity,
        })
    }
    fn commit_selection(
        &mut self,
        command: &SelectRuntimeRevision,
        previous: &SelectedRevision,
    ) -> Result<String, Self::Error> {
        let profile = self.runtime_profile(&command.profile_id)?;
        if profile.resource_version != previous.profile_version
            || profile.active_revision_id.as_deref() != Some(previous.revision_id.as_str())
        {
            return Err(CleanStoreError::StaleVersion);
        }
        let revision = self.runtime_profile_revision(&previous.revision_id)?;
        let Some(v1::runtime_profile_revision::Configuration::AyniManaged(mut configuration)) =
            revision.configuration
        else {
            return Err(CleanStoreError::InvalidTransition);
        };
        let snapshot = self.observe_repository_snapshot(
            &profile.repository_connection_id,
            command.source_revision.as_str(),
        )?;
        configuration
            .lock_fingerprint
            .clone_from(&command.identity.policy_binding);
        configuration
            .oci_repository
            .clone_from(&command.identity.image_repository);
        configuration
            .ayni_version
            .clone_from(&command.policy_version);
        configuration
            .runtime_source_digest
            .clone_from(&command.identity.source_digest);
        Ok(self
            .record_selected_runtime_revision(
                &command.profile_id,
                &snapshot.id,
                v1::runtime_profile_revision::Configuration::AyniManaged(configuration),
                false,
            )?
            .id)
    }
}
