import {describe,it,expect} from "vitest";
import {FrozenAgentProfile} from "@ayni-orchestrator/grpc-bindings/server";
import {mapFrozenProfile} from "../src/agent-profiles.js";
describe("agent profile browser boundary",()=>{
 it("returns pinned capabilities and binding identifiers without credential material",()=>{
  const wire=FrozenAgentProfile.fromPartial({revision:`sha256:${"a".repeat(64)}`,policyRevision:"builtin-role-v1",role:"planner",provider:"openai",connectionId:"connection:1",credentialRevision:"revision-1",model:"model",repositoryIds:["repo:1"],settings:{instructions:"Read only",skills:[{id:"review",revision:"v1",instructions:"Check compatibility"}],allowedTools:["read_repository_file"],maxModelCalls:4,deadlineSeconds:"300"}});
  const dirty={...wire,apiKey:"SECRET_SENTINEL",secretName:"SECRET_SENTINEL"};
  expect(mapFrozenProfile(dirty)).toMatchObject({policyRevision:"builtin-role-v1",settings:{maxModelCalls:4,deadlineSeconds:"300",skills:[{id:"review",revision:"v1",instructions:"Check compatibility"}]}});
  expect(JSON.stringify(mapFrozenProfile(dirty))).not.toContain("SECRET_SENTINEL");
 });
 it("fails closed for missing revision, role and settings",()=>{
  for(const value of [{},{revision:"unknown",role:"planner",provider:"openai",settings:{}},{revision:`sha256:${"a".repeat(64)}`,role:"administrator",provider:"openai",settings:{}}])expect(()=>mapFrozenProfile(FrozenAgentProfile.fromPartial(value))).toThrow();
 });
});
