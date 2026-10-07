//! Replay-safe automation setup admission with profile-owned planning and atomic persistence.
use crate::tasks::RequestIdentity;
use thiserror::Error;
pub struct InstallAutomation {
    pub identity: RequestIdentity,
    pub profile_id: String,
}
pub struct Facts {
    pub profile_version: u64,
    pub revision_id: String,
    pub repository_id: String,
    pub repository_version: u64,
    pub default_branch: String,
    pub installation_id: u64,
    pub template_version: Option<String>,
}
pub struct Plan {
    pub version: String,
    pub environment_workflow_path: String,
}
pub trait Policy {
    type Error: std::error::Error;
    /// Plan and validate controller-owned integration files without credentials or mutations.
    /// # Errors
    /// Rejects unsupported template and unsafe setup identity.
    fn plan(&self, profile_id: &str, facts: &Facts) -> Result<Plan, Self::Error>;
}
pub trait Repository {
    type Error: std::error::Error;
    /// # Errors
    /// Rejects conflicting request identities and corrupt/unavailable retained results.
    fn replay_automation(&self, identity: &RequestIdentity) -> Result<Option<String>, Self::Error>;
    /// # Errors
    /// Rejects missing, corrupt or unavailable profile/repository facts.
    fn automation_facts(&self, profile_id: &str) -> Result<Facts, Self::Error>;
    /// Commit installation, pending action and event atomically with observed-version checks.
    /// # Errors
    /// Rejects stale facts and persistence failure without partial intent/events.
    fn commit_automation(
        &mut self,
        command: &InstallAutomation,
        facts: &Facts,
        plan: &Plan,
    ) -> Result<String, Self::Error>;
}
#[derive(Debug, Error)]
pub enum AdmissionError<S, P> {
    #[error("automation persistence failed")]
    Storage(#[source] S),
    #[error("automation planning failed")]
    Policy(#[source] P),
    #[error("runtime profile does not support this automation")]
    UnsupportedProfile,
}
/// Replay precedes current revision policy; a new request records intent before external work.
/// # Errors
/// Rejects unsupported profile, invalid planning and persistence failure.
pub fn install<R: Repository, P: Policy>(
    repository: &mut R,
    policy: &P,
    command: &InstallAutomation,
) -> Result<String, AdmissionError<R::Error, P::Error>> {
    if let Some(id) = repository
        .replay_automation(&command.identity)
        .map_err(AdmissionError::Storage)?
    {
        return Ok(id);
    }
    let facts = repository
        .automation_facts(&command.profile_id)
        .map_err(AdmissionError::Storage)?;
    if facts.template_version.is_none() {
        return Err(AdmissionError::UnsupportedProfile);
    }
    let plan = policy
        .plan(&command.profile_id, &facts)
        .map_err(AdmissionError::Policy)?;
    repository
        .commit_automation(command, &facts, &plan)
        .map_err(AdmissionError::Storage)
}
#[cfg(test)]
mod tests;
