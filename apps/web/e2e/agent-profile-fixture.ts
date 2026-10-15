import {createHash} from "node:crypto";
import {FrozenAgentProfile,type ProjectAgentDefaults} from "@ayni-orchestrator/grpc-bindings/server";
import {capabilities} from "../fixtures/agent-capabilities.js";
export {capabilities};
import {type ProfileRole} from "../src/lib/agent-profiles.js";
export function effectiveProfiles(project:ProjectAgentDefaults|undefined,workspace:ProjectAgentDefaults|undefined):ProjectAgentDefaults {
 const defaults={projectConnectionId:"",projectModel:"",codingConnectionId:"",codingModel:"",...workspace};
 for(const role of ["project","coding"] as const)if(project?.[`${role}ConnectionId`]){defaults[`${role}ConnectionId`]=project[`${role}ConnectionId`];defaults[`${role}Model`]=project[`${role}Model`];}
 for(const role of ["planner","worker"] as const){const base=workspace?.[`${role}Profile`]??capabilities.roles[role],override=project?.[`${role}Profile`];defaults[`${role}Profile`]=override?{...override,instructions:[base.instructions,override.instructions].filter(Boolean).join("\n")}:base;}
 return defaults;
}
export function freezeProfile(defaults:NonNullable<ReturnType<typeof effectiveProfiles>>,role:ProfileRole,repositoryIds:string[]){
 const profile=FrozenAgentProfile.fromPartial({policyRevision:"builtin-role-v1",systemInstructions:"Fixture: role tools remain bounded by application policy.",role,provider:"openai",connectionId:role==="planner"?defaults.projectConnectionId:defaults.codingConnectionId,credentialRevision:"fixture-revision",model:role==="planner"?defaults.projectModel:defaults.codingModel,repositoryIds,settings:(role==="planner"?defaults.plannerProfile:defaults.workerProfile)??capabilities.roles[role]});
 profile.revision=`sha256:${createHash("sha256").update(JSON.stringify(profile)).digest("hex")}`;
 return profile;
}
