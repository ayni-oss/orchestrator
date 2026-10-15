import {describe,it,expect} from "vitest";
import {ProjectWorkerUpdate} from "@ayni-orchestrator/grpc-bindings/server";
import {mapWorkerUpdate} from "../src/worker-evidence.js";
const origin={projectId:"project:1",conversationId:"conversation:1",planId:"plan:1",planVersion:"9007199254740993",proposalId:"proposal:1",researchRunId:"run:1"};
function update(){return ProjectWorkerUpdate.fromPartial({origin,taskId:"task:1",attemptId:"attempt:1",attemptNumber:2,resourceVersion:"9007199254740994",state:4,result:{execution:1,commit:"a".repeat(40),changedPaths:["src/lib.rs"],quality:{outcome:2,findings:["Tests failed"],evidence:{artifactPath:"private-worker-path",signalsDigest:"b".repeat(64),sourceFingerprint:"c".repeat(64),commit:"a".repeat(40)}},publication:3,pullRequest:{url:"https://github.com/ayni-oss/sandbox/pull/42",number:"42",draft:true}},updatedAt:new Date("2026-10-15T00:00:00Z")});}
describe("accepted worker evidence projection",()=>{
 it("retains exact provenance and independently reports failed quality on a published draft",()=>{
  const dto=mapWorkerUpdate(update(),origin.projectId,origin.conversationId);
  expect(dto.origin).toEqual(origin);expect(dto.resourceVersion).toBe("9007199254740994");
  expect(dto.result).toMatchObject({execution:"succeeded",quality:{outcome:"failed",findings:["Tests failed"]},publication:"published",pullRequest:{draft:true}});
  expect(JSON.stringify(dto)).not.toContain("private-worker-path");
 });
 it("fails closed for another project, conversation, or invalid plan version",()=>{
  expect(()=>mapWorkerUpdate(update(),"project:other",origin.conversationId)).toThrow();
  expect(()=>mapWorkerUpdate(update(),origin.projectId,"conversation:other")).toThrow();
  const bad=update();bad.origin!.planVersion="0";expect(()=>mapWorkerUpdate(bad,origin.projectId,origin.conversationId)).toThrow();
 });
 it("treats missing quality evidence as unavailable and drops unsafe receipt links",()=>{
  const value=update();value.result!.quality={outcome:1,rows:[],findings:[]};value.result!.pullRequest!.url="javascript:alert(1)";
  const dto=mapWorkerUpdate(value,origin.projectId,origin.conversationId);
  expect(dto.result?.quality?.outcome).toBe("unavailable");expect(dto.result?.pullRequest?.url).toBeUndefined();
 });
});
