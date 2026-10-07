//! Automation persistence codec and atomic intent/event mutation.
use super::{
    CleanStore, CleanStoreError, Message, PROFILE_SERVICE, action_row, append_event,
    map_constraint, params, resource_id, timestamp, unix_seconds, v1,
};
use crate::profiles::ayni::automation::PlanningError;
use ayni_orchestrator_application::{
    automation::{AdmissionError, Facts, InstallAutomation, Plan, Repository},
    tasks::RequestIdentity,
};
pub(crate) fn admission_error(
    error: AdmissionError<CleanStoreError, PlanningError>,
) -> CleanStoreError {
    match error {
        AdmissionError::Storage(error) => error,
        AdmissionError::UnsupportedProfile => CleanStoreError::InvalidTransition,
        AdmissionError::Policy(PlanningError::UnsupportedTemplate(version)) => {
            CleanStoreError::UnsupportedWorkflowTemplate(version)
        }
        AdmissionError::Policy(PlanningError::InvalidIdentity) => CleanStoreError::CorruptState,
    }
}
impl Repository for CleanStore {
    type Error = CleanStoreError;
    fn replay_automation(&self, identity: &RequestIdentity) -> Result<Option<String>, Self::Error> {
        let Some(action) = action_row(
            &self.connection,
            "InstallRepositoryAutomation",
            &identity.key,
        )?
        else {
            return Ok(None);
        };
        if action.request_hash != identity.digest {
            return Err(CleanStoreError::IdempotencyConflict);
        }
        let id = action.target_id.ok_or(CleanStoreError::CorruptState)?;
        self.automation_installation(&id)?;
        Ok(Some(id))
    }
    fn automation_facts(&self, profile_id: &str) -> Result<Facts, Self::Error> {
        let profile = self.runtime_profile(profile_id)?;
        let repository = self.repository_connection(&profile.repository_connection_id)?;
        let github = repository
            .provider
            .and_then(|p| p.provider)
            .map(|p| match p {
                v1::repository_provider::Provider::Github(value) => value,
            })
            .ok_or(CleanStoreError::CorruptState)?;
        let revision_id = profile
            .active_revision_id
            .ok_or(CleanStoreError::CorruptState)?;
        let revision = self.runtime_profile_revision(&revision_id)?;
        let template_version = match revision.configuration {
            Some(v1::runtime_profile_revision::Configuration::AyniManaged(value)) => {
                Some(value.workflow_template_version)
            }
            _ => None,
        };
        Ok(Facts {
            profile_version: profile.resource_version,
            revision_id,
            repository_id: repository.id,
            repository_version: repository.resource_version,
            default_branch: repository.default_branch,
            installation_id: github.installation_id,
            template_version,
        })
    }
    fn commit_automation(
        &mut self,
        command: &InstallAutomation,
        facts: &Facts,
        plan: &Plan,
    ) -> Result<String, Self::Error> {
        let profile = self.runtime_profile(&command.profile_id)?;
        let repository = self.repository_connection(&facts.repository_id)?;
        if profile.resource_version != facts.profile_version
            || profile.active_revision_id.as_deref() != Some(facts.revision_id.as_str())
            || profile.repository_connection_id != facts.repository_id
            || repository.resource_version != facts.repository_version
        {
            return Err(CleanStoreError::StaleVersion);
        }
        let now = unix_seconds()?;
        let id = resource_id("automation");
        let installation = v1::AutomationInstallation {
            id: id.clone(),
            runtime_profile_id: profile.id.clone(),
            version: plan.version.clone(),
            state: v1::AutomationInstallationState::SetupRequired as i32,
            provider: Some(v1::automation_installation::Provider::Github(
                v1::GithubAutomationInstallation {
                    installation_id: facts.installation_id,
                    setup_pull_request_number: None,
                    environment_workflow_path: plan.environment_workflow_path.clone(),
                },
            )),
            failure: None,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('InstallRepositoryAutomation',?1,?2,?3,'pending','automation_installation',?4,?5,?5)",
                params![command.identity.key, command.identity.digest, command.identity.payload, id, now],
            )
            .map_err(map_constraint)?;
        let cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "automation_installation",
            &id,
            1,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &installation,
        )?;
        transaction
            .execute(
                "INSERT INTO automation_installations(
                    id,runtime_profile_id,state,version,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,1,?6)",
                params![
                    id,
                    profile.id,
                    installation.state,
                    plan.version,
                    installation.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(installation.id)
    }
}
