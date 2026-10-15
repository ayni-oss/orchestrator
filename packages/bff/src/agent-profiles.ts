import type {FrozenAgentProfile,AgentProfileSettings} from "@ayni-orchestrator/grpc-bindings/server";
import type {FrozenAgentProfileDto,AgentProfileSettingsDto} from "./dto.js";
import {BffMappingError} from "./mapping.js";
export function mapProfileSettings(value:AgentProfileSettings):AgentProfileSettingsDto {
  return {instructions:value.instructions,skills:value.skills.map(s=>({id:s.id,revision:s.revision,instructions:s.instructions})),allowedTools:[...value.allowedTools],repositoryIds:[...value.repositoryIds],maxModelCalls:value.maxModelCalls,deadlineSeconds:value.deadlineSeconds};
}
export function mapFrozenProfile(value:FrozenAgentProfile):FrozenAgentProfileDto {
  if(!/^sha256:[a-f0-9]{64}$/u.test(value.revision)||!value.settings||!["planner","worker"].includes(value.role)||value.provider!=="openai")throw new BffMappingError();
  return {revision:value.revision,policyRevision:value.policyRevision,systemInstructions:value.systemInstructions,role:value.role,provider:value.provider,connectionId:value.connectionId,credentialRevision:value.credentialRevision,model:value.model,repositoryIds:[...value.repositoryIds],settings:mapProfileSettings(value.settings)};
}
