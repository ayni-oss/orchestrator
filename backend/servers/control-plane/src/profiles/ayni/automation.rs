//! Controller managed workflow planning before setup intent is persisted.
use super::onboarding_templates;
use crate::adapters::github::setup;
use ayni_orchestrator_application::automation::{Facts, Plan, Policy};
use thiserror::Error;
pub struct ManagedAutomation;
#[derive(Debug, Error)]
pub enum PlanningError {
    #[error("unsupported managed workflow template {0:?}")]
    UnsupportedTemplate(String),
    #[error("invalid managed setup identity")]
    InvalidIdentity,
}
impl Policy for ManagedAutomation {
    type Error = PlanningError;
    fn plan(&self, profile_id: &str, facts: &Facts) -> Result<Plan, Self::Error> {
        let version = facts
            .template_version
            .as_deref()
            .ok_or(PlanningError::InvalidIdentity)?;
        let files = onboarding_templates::render(version, &facts.default_branch)
            .map_err(|_| PlanningError::UnsupportedTemplate(version.into()))?;
        setup::setup_pull_request(profile_id, &facts.default_branch, files)
            .map_err(|_| PlanningError::InvalidIdentity)?;
        Ok(Plan {
            version: version.into(),
            environment_workflow_path: onboarding_templates::ENVIRONMENT_WORKFLOW_PATH.into(),
        })
    }
}
