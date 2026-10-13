import type { ProjectConversation } from "@ayni-orchestrator/grpc-bindings/server";
import type { ProjectConversationDto } from "./dto.js";
import { BffMappingError } from "./mapping.js";

export type ProjectConversationResponse = { conversation?: ProjectConversation | undefined };

function required<T>(value: T | null | undefined): T {
  if (value === undefined || value === null) throw new BffMappingError();
  return value;
}

// Project-agent credentials are never reflected into browser conversation data.
export function mapConversation(value:ProjectConversation):ProjectConversationDto {
  const proposals=value.proposals.map(proposal=>{
    const request=proposal.request;
    return {
      id:proposal.id,runId:proposal.runId,state:proposal.state,action:proposal.action,
      targetTaskId:proposal.targetTaskId,taskId:proposal.taskId,attemptId:proposal.attemptId,failure:proposal.failure,
      ...(proposal.researchedRuntime ? {runtime:{profileRevisionId:proposal.researchedRuntime.runtimeProfileRevisionId,artifactId:proposal.researchedRuntime.runtimeArtifactId,image:proposal.researchedRuntime.ociRepository,digest:proposal.researchedRuntime.resolvedDigest}} : {}),
      ...(request ? {request:{
        projectId:request.projectId,repositoryConnectionId:request.repositoryConnectionId,
        repositoryRevision:request.repositoryRevision,runtimeProfileId:request.runtimeProfileId,
        connectionId:request.connectionId,agent:required(request.agent),request:request.request,
        acceptanceCriteria:[...request.acceptanceCriteria],
        changeConstraints:request.changeConstraints ? {protectedPaths:[...request.changeConstraints.protectedPaths],allowTestEdits:request.changeConstraints.allowTestEdits} : {protectedPaths:[],allowTestEdits:true},
        ...(request.codingTurns===undefined ? {} : {codingTurns:request.codingTurns}),
        ...(request.deadline ? {deadlineSeconds:Number(request.deadline.seconds)} : {}),
      }} : {}),
    };
  });
  return {id:value.id,projectId:value.projectId,title:value.title,repositoryConnectionIds:[...value.repositoryConnectionIds],updateCursor:value.updateCursor,createdAt:required(value.createdAt).toISOString(),
    messages:value.messages.map(m=>({id:m.id,role:m.role,content:m.content,createdAt:required(m.createdAt).toISOString(),runId:m.runId})),
    runs:value.runs.map(r=>({id:r.id,state:r.state,model:r.model,connectionId:r.connectionId,modelCalls:r.modelCalls,failure:r.failure,...(r.tokenUsage ? {usage:{inputTokens:r.tokenUsage.inputTokens,outputTokens:r.tokenUsage.outputTokens,totalTokens:r.tokenUsage.totalTokens}} : {})})),
    proposals,
  };
}
