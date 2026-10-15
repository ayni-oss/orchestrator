//! Freeze profiles at admission; workspace authority bounds project selections.
use super::{CleanStore, CleanStoreError, v1};
use crate::agent_profiles::{self, selection};
use ayni_orchestrator_runtime::agent_profile::{FrozenProfile, Role};

impl CleanStore {
    pub(super) fn merge_agent_profiles(
        &self,
        workspace: Option<&v1::ProjectAgentDefaults>,
        project: Option<&v1::ProjectAgentDefaults>,
        effective: &mut v1::ProjectAgentDefaults,
    ) -> Result<(), CleanStoreError> {
        for role in [Role::Planner, Role::Worker] {
            let base = selection(workspace.and_then(|d| profile_setting(d, &role)), &role)
                .map_err(|_| CleanStoreError::InvalidTransition)?;
            let profile = if let Some(selected) = project.and_then(|d| profile_setting(d, &role)) {
                base.narrow(&agent_profiles::settings(selected), &role)
                    .map_err(|_| CleanStoreError::InvalidTransition)?
            } else {
                base
            };
            match role {
                Role::Planner => {
                    effective.planner_profile = Some(agent_profiles::wire_settings(&profile));
                }
                Role::Worker => {
                    effective.worker_profile = Some(agent_profiles::wire_settings(&profile));
                }
            }
        }
        Ok(())
    }
    pub(super) fn freeze_run_profiles(
        &self,
        c: &v1::ProjectConversation,
        run: &mut v1::ProjectAgentRun,
    ) -> Result<(), CleanStoreError> {
        let Some(binding) = &run.provider_credentials else {
            return Ok(());
        };
        let defaults = self
            .effective_project_defaults(&self.project(&c.project_id)?)?
            .ok_or(CleanStoreError::InvalidTransition)?;
        if run.model != defaults.project_model
            || binding.connection_id != defaults.project_connection_id
            || self.freeze_task_provider(&binding.connection_id)?.as_ref() != Some(binding)
        {
            return Err(CleanStoreError::InvalidTransition);
        }
        let coding = self
            .freeze_task_provider(&defaults.coding_connection_id)?
            .ok_or(CleanStoreError::InvalidTransition)?;
        let freeze = |role: Role,
                      connection: String,
                      revision: String,
                      model: String,
                      settings: Option<&v1::AgentProfileSettings>| {
            FrozenProfile::freeze(
                role.clone(),
                connection,
                revision,
                model,
                selection(settings, &role).map_err(|_| CleanStoreError::InvalidTransition)?,
                c.repository_connection_ids.clone(),
            )
            .map(|p| agent_profiles::wire(&p))
            .map_err(|_| CleanStoreError::InvalidTransition)
        };
        run.planner_profile = Some(freeze(
            Role::Planner,
            binding.connection_id.clone(),
            binding.credential_revision.clone(),
            run.model.clone(),
            defaults.planner_profile.as_ref(),
        )?);
        run.worker_profile = Some(freeze(
            Role::Worker,
            coding.connection_id,
            coding.credential_revision,
            defaults.coding_model,
            defaults.worker_profile.as_ref(),
        )?);
        Ok(())
    }
    pub(super) fn freeze_worker_profile(
        &self,
        project: &v1::Project,
        repository: &str,
        model: &str,
        binding: Option<&v1::FrozenProviderCredentials>,
        origin: Option<&ayni_orchestrator_application::tasks::TaskOrigin>,
    ) -> Result<v1::FrozenAgentProfile, CleanStoreError> {
        let defaults = self
            .effective_project_defaults(project)?
            .unwrap_or_default();
        let settings = selection(defaults.worker_profile.as_ref(), &Role::Worker)
            .map_err(|_| CleanStoreError::InvalidTransition)?;
        let profile = FrozenProfile::freeze(
            Role::Worker,
            binding.map_or_else(String::new, |b| b.connection_id.clone()),
            binding.map_or_else(String::new, |b| b.credential_revision.clone()),
            model.into(),
            settings,
            vec![repository.into()],
        )
        .map_err(|_| CleanStoreError::InvalidTransition)?;
        if let Some(origin) = origin {
            let c = self.project_conversation(&origin.conversation_id)?;
            let run = c
                .runs
                .iter()
                .find(|r| r.id == origin.research_run_id)
                .ok_or(CleanStoreError::CorruptState)?;
            if let Some(expected) = &run.worker_profile {
                let expected =
                    agent_profiles::frozen(expected).map_err(|_| CleanStoreError::CorruptState)?;
                if expected.settings != profile.settings
                    || expected.connection_id != profile.connection_id
                    || expected.credential_revision != profile.credential_revision
                    || expected.model != profile.model
                    || !expected.repository_ids.contains(&repository.to_owned())
                {
                    return Err(CleanStoreError::InvalidTransition);
                }
            }
        }
        Ok(agent_profiles::wire(&profile))
    }
}

const fn profile_setting<'a>(
    defaults: &'a v1::ProjectAgentDefaults,
    role: &Role,
) -> Option<&'a v1::AgentProfileSettings> {
    match role {
        Role::Planner => defaults.planner_profile.as_ref(),
        Role::Worker => defaults.worker_profile.as_ref(),
    }
}

pub(super) fn migrate(connection: &super::Connection) -> Result<(), CleanStoreError> {
    // Fence older binaries which cannot enforce persisted capability selections.
    connection
        .execute_batch("UPDATE clean_schema SET version=25 WHERE component='control_plane'")
        .map_err(CleanStoreError::Database)
}
impl CleanStore {
    pub(crate) fn validate_profile_connection(
        &self,
        profile: &v1::FrozenAgentProfile,
    ) -> Result<(), CleanStoreError> {
        let profile = agent_profiles::frozen(profile).map_err(|_| CleanStoreError::CorruptState)?;
        let connection = self.get_connection(&profile.connection_id)?;
        if !connection.enabled
            || connection.status != "ready"
            || connection.kind != profile.provider
        {
            return Err(CleanStoreError::InvalidTransition);
        }
        Ok(())
    }
}

pub(super) fn validate_profile_scopes(
    defaults: Option<&v1::ProjectAgentDefaults>,
    members: &[String],
) -> Result<(), CleanStoreError> {
    if let Some(defaults) = defaults {
        for profile in [&defaults.planner_profile, &defaults.worker_profile]
            .into_iter()
            .flatten()
        {
            if profile
                .repository_ids
                .iter()
                .any(|id| !members.contains(id))
            {
                return Err(CleanStoreError::ParentMismatch);
            }
        }
    }
    Ok(())
}
