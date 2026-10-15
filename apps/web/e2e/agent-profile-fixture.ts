import {createHash} from "node:crypto";
import {FrozenAgentProfile,type ProjectAgentDefaults} from "@ayni-orchestrator/grpc-bindings/server";
import {effectiveAgentDefaults} from "../src/lib/server/agent-defaults.js";
import {profileDefaults,type ProfileRole} from "../src/lib/agent-profiles.js";
export function effectiveProfiles(project:ProjectAgentDefaults|undefined,workspace:ProjectAgentDefaults|undefined){return effectiveAgentDefaults(project,workspace);}
export function freezeProfile(defaults:NonNullable<ReturnType<typeof effectiveProfiles>>,role:ProfileRole,repositoryIds:string[]){
 const profile=FrozenAgentProfile.fromPartial({policyRevision:"builtin-role-v1",systemInstructions:"Fixture: role tools remain bounded by application policy.",role,provider:"openai",connectionId:role==="planner"?defaults.projectConnectionId:defaults.codingConnectionId,credentialRevision:"fixture-revision",model:role==="planner"?defaults.projectModel:defaults.codingModel,repositoryIds,settings:(role==="planner"?defaults.plannerProfile:defaults.workerProfile)??profileDefaults(role)});
 profile.revision=`sha256:${createHash("sha256").update(JSON.stringify(profile)).digest("hex")}`;
 return profile;
}
