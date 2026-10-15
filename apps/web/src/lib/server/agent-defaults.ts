import type {ProjectAgentDefaultsDto} from "@ayni-orchestrator/bff/server";
/** A connection and model form one override; blank pairs inherit workspace defaults. */
export function effectiveAgentDefaults(overrides:ProjectAgentDefaultsDto|undefined,workspace:ProjectAgentDefaultsDto|undefined):ProjectAgentDefaultsDto|undefined {
  const project=overrides?.projectConnectionId?overrides:workspace;
  const coding=overrides?.codingConnectionId?overrides:workspace;
  if(!project?.projectConnectionId&&!coding?.codingConnectionId)return undefined;
  return {projectConnectionId:project?.projectConnectionId??"",projectModel:project?.projectModel??"",codingConnectionId:coding?.codingConnectionId??"",codingModel:coding?.codingModel??""};
}
