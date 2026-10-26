import {ProjectWorkerUpdate,type ProjectConversation} from "@ayni-orchestrator/grpc-bindings/server";

export function setWorkerEvidence(c:ProjectConversation,t:any,scenario:string,version?:string){
  const previous=c.workerUpdates.find(update=>update.taskId===t.id);
  const resourceVersion=version??String(Number(previous?.resourceVersion??0)+1);
  if(previous&&BigInt(previous.resourceVersion)>=BigInt(resourceVersion))return false;
  const question=scenario==="input_required"?"Which API compatibility behavior should be preserved?":undefined;
  const quality=scenario==="failed_quality"?2:scenario==="unavailable"||question||scenario==="cancelled"||scenario==="failed"?3:1;
  const publication=scenario==="publication_failed"?4:question||scenario==="cancelled"||scenario==="failed"?6:3;
  const execution=question?6:scenario==="cancelled"?3:scenario==="failed"?2:1;
  const state=question?3:scenario==="cancelled"?6:scenario==="failed"?5:4;
  const commit="d".repeat(40);
  const origin=t.snapshot.origin;
  const receipt=publication===3?{repository:"ayni-oss/sandbox",number:"9",url:"https://github.com/ayni-oss/sandbox/pull/9",branch:"task/example",baseBranch:"main",headCommit:commit,draft:true}:undefined;
  const result={execution,commit,changedPaths:["src/main.rs"],quality:{outcome:quality,findings:quality===2?["Repair the failing compatibility test"]:[],evidence:quality===3?undefined:{artifactPath:"private-evidence-path",signalsDigest:"e".repeat(64),sourceFingerprint:"f".repeat(64),commit}},publication,pullRequest:receipt,requestedInput:question};
  const update=ProjectWorkerUpdate.fromPartial({origin,taskId:t.id,attemptId:`attempt:${t.id}`,attemptNumber:1,resourceVersion,state,requestedInput:question,result,pullRequest:receipt,updatedAt:new Date("2026-10-15T00:00:00Z")});
  if(scenario==="progress"){update.state=2;update.result=undefined;update.pullRequest=undefined;update.progress={phase:4,codingTurnsConsumed:4,qualityRound:1,publication:1};}
  if(scenario==="publication_retry"){update.publicationOnly=true;update.state=1;update.result=previous?.result;update.pullRequest=undefined;}
  const index=c.workerUpdates.findIndex(value=>value.attemptId===update.attemptId);
  if(index<0)c.workerUpdates.push(update);else c.workerUpdates[index]=update;
  return true;
}
