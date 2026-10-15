//! Wire conversion for credential-free, application-owned agent capabilities.
use ayni_orchestrator_api::v1;
use ayni_orchestrator_runtime::agent_profile::{FrozenProfile, Role, Settings, SkillRevision};
use std::io;

pub fn settings(value: &v1::AgentProfileSettings) -> Settings {
    Settings {
        instructions: value.instructions.clone(),
        skills: value
            .skills
            .iter()
            .map(|s| SkillRevision {
                id: s.id.clone(),
                revision: s.revision.clone(),
                instructions: s.instructions.clone(),
            })
            .collect(),
        allowed_tools: value.allowed_tools.clone(),
        repository_ids: value.repository_ids.clone(),
        max_model_calls: value.max_model_calls,
        deadline_seconds: value.deadline_seconds,
    }
}
pub fn wire_settings(value: &Settings) -> v1::AgentProfileSettings {
    v1::AgentProfileSettings {
        instructions: value.instructions.clone(),
        skills: value
            .skills
            .iter()
            .map(|s| v1::AgentSkillRevision {
                id: s.id.clone(),
                revision: s.revision.clone(),
                instructions: s.instructions.clone(),
            })
            .collect(),
        allowed_tools: value.allowed_tools.clone(),
        repository_ids: value.repository_ids.clone(),
        max_model_calls: value.max_model_calls,
        deadline_seconds: value.deadline_seconds,
    }
}
pub fn wire(value: &FrozenProfile) -> v1::FrozenAgentProfile {
    v1::FrozenAgentProfile {
        revision: value.revision.clone(),
        policy_revision: value.policy_revision.clone(),
        system_instructions: value.system_instructions.clone(),
        role: match value.role {
            Role::Planner => "planner",
            Role::Worker => "worker",
        }
        .into(),
        provider: value.provider.clone(),
        connection_id: value.connection_id.clone(),
        credential_revision: value.credential_revision.clone(),
        model: value.model.clone(),
        settings: Some(wire_settings(&value.settings)),
        repository_ids: value.repository_ids.clone(),
    }
}
pub fn frozen(value: &v1::FrozenAgentProfile) -> io::Result<FrozenProfile> {
    let profile = FrozenProfile {
        revision: value.revision.clone(),
        policy_revision: value.policy_revision.clone(),
        system_instructions: value.system_instructions.clone(),
        role: match value.role.as_str() {
            "planner" => Role::Planner,
            "worker" => Role::Worker,
            _ => return Err(io::Error::other("invalid agent role")),
        },
        provider: value.provider.clone(),
        connection_id: value.connection_id.clone(),
        credential_revision: value.credential_revision.clone(),
        model: value.model.clone(),
        settings: settings(
            value
                .settings
                .as_ref()
                .ok_or_else(|| io::Error::other("profile settings missing"))?,
        ),
        repository_ids: value.repository_ids.clone(),
    };
    profile.validate(&profile.role, &profile.model, &profile.repository_ids)?;
    Ok(profile)
}
pub fn selection(value: Option<&v1::AgentProfileSettings>, role: &Role) -> io::Result<Settings> {
    let result = value.map_or_else(|| Settings::defaults(role), settings);
    result.validate(role)?;
    Ok(result)
}

pub fn validate_snapshot(snapshot: &v1::DevelopmentSnapshot) -> io::Result<()> {
    if let Some(profile) = &snapshot.agent_profile {
        let profile = frozen(profile)?;
        let repository = snapshot
            .repository
            .as_ref()
            .ok_or_else(|| io::Error::other("repository missing"))?;
        let agent = snapshot
            .agent
            .as_ref()
            .ok_or_else(|| io::Error::other("model missing"))?;
        profile.validate(
            &Role::Worker,
            &agent.model,
            std::slice::from_ref(&repository.repository_connection_id),
        )?;
        if snapshot
            .limits
            .as_ref()
            .is_none_or(|l| l.coding_turns > profile.settings.max_model_calls)
            || snapshot.provider_credentials.as_ref().is_some_and(|b| {
                b.connection_id != profile.connection_id
                    || b.credential_revision != profile.credential_revision
            })
        {
            return Err(io::Error::other("profile binding mismatch"));
        }
    }
    Ok(())
}
pub fn validate_run(c: &v1::ProjectConversation, run: &v1::ProjectAgentRun) -> io::Result<()> {
    if let Some(profile) = &run.planner_profile {
        let profile = frozen(profile)?;
        profile.validate(&Role::Planner, &run.model, &c.repository_connection_ids)?;
        if profile.connection_id != run.connection_id
            || profile.credential_revision != run.credential_revision
        {
            return Err(io::Error::other("profile binding mismatch"));
        }
    }
    if let Some(profile) = &run.worker_profile {
        let profile = frozen(profile)?;
        profile.validate(&Role::Worker, &profile.model, &c.repository_connection_ids)?;
    }
    Ok(())
}
