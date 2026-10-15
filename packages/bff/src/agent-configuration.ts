import type {AgentProfileServiceClient,GetAgentCapabilitiesResponse} from "@ayni-orchestrator/grpc-bindings/server";
import type {AgentCapabilitiesDto,RequestContext} from "./dto.js";
import type {UnaryInvoker} from "./workspaces.js";
import {mapProfileSettings} from "./agent-profiles.js";
import {BffMappingError} from "./mapping.js";
function required<T>(value:T|undefined):T {if(value===undefined)throw new BffMappingError();return value;}
export async function readAgentCapabilities(client:AgentProfileServiceClient,unary:<T>(context:RequestContext,invoke:UnaryInvoker<T>)=>Promise<T>,context:RequestContext):Promise<AgentCapabilitiesDto> {
    const response=await unary<GetAgentCapabilitiesResponse>(context,(m,o,c)=>client.getAgentCapabilities({},m,o,c));
    const limits=required(response.limits);
    const roles={} as AgentCapabilitiesDto["roles"];
    for(const role of ["planner","worker"] as const){
      const entries=response.roles.filter(value=>value.role===role);
      if(entries.length!==1)throw new BffMappingError();
      roles[role]=mapProfileSettings(required(entries[0]!.defaults));
    }
    if(!limits.maxInstructionBytes||!limits.maxSkills||!limits.maxRepositories||!limits.maxModelCalls||!limits.maxIdentifierBytes||!/^\d+$/u.test(limits.maxDeadlineSeconds)||BigInt(limits.maxDeadlineSeconds)<1n)throw new BffMappingError();
    return {roles,limits:{...limits}};
  }
