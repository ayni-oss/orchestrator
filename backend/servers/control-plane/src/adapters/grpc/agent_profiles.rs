//! Read-only capabilities, derived from the same contract used at admission.
use super::{GrpcApi, Request, Response, Status, v1};
use ayni_orchestrator_runtime::agent_profile::{self, Role, Settings};

#[tonic::async_trait]
impl v1::agent_profile_service_server::AgentProfileService for GrpcApi {
    async fn get_agent_capabilities(
        &self,
        request: Request<v1::GetAgentCapabilitiesRequest>,
    ) -> Result<Response<v1::GetAgentCapabilitiesResponse>, Status> {
        let context = self.context(&request, "GetAgentCapabilities")?;
        Ok(context.response(v1::GetAgentCapabilitiesResponse {
            roles: [Role::Planner, Role::Worker]
                .iter()
                .map(|role| v1::AgentRoleCapabilities {
                    role: match role {
                        Role::Planner => "planner",
                        Role::Worker => "worker",
                    }
                    .into(),
                    defaults: Some(crate::agent_profiles::wire_settings(&Settings::defaults(
                        role,
                    ))),
                })
                .collect(),
            limits: Some(v1::AgentProfileLimits {
                max_instruction_bytes: u32::try_from(agent_profile::MAX_INSTRUCTION_BYTES)
                    .unwrap_or(u32::MAX),
                max_skills: u32::try_from(agent_profile::MAX_SKILLS).unwrap_or(u32::MAX),
                max_repositories: u32::try_from(agent_profile::MAX_REPOSITORIES)
                    .unwrap_or(u32::MAX),
                max_model_calls: agent_profile::MAX_MODEL_CALLS,
                max_deadline_seconds: agent_profile::MAX_DEADLINE_SECONDS,
                max_identifier_bytes: u32::try_from(agent_profile::MAX_IDENTIFIER_BYTES)
                    .unwrap_or(u32::MAX),
            }),
        }))
    }
}
