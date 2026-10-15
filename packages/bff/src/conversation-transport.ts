import type {ProjectConversationServiceClient,ListProjectConversationsResponse,GetProjectConversationContextResponse,ListProjectConversationHistoryResponse} from "@ayni-orchestrator/grpc-bindings/server";
import type {ProjectConversationDto,RequestContext,ConversationContextDto,ConversationHistoryDto,ConversationHistoryInput} from "./dto.js";
import type {UnaryInvoker} from "./workspaces.js";
import {mapConversation,type ProjectConversationResponse} from "./conversations.js";
import {BffMappingError} from "./mapping.js";
type Unary=<T>(context:RequestContext,invoke:UnaryInvoker<T>)=>Promise<T>;
function required<T>(value:T|undefined):T{if(value===undefined)throw new BffMappingError();return value;}
export function conversationOperations(client:ProjectConversationServiceClient,unary:Unary){return {
  async getProjectConversationContext(input:{projectId:string;conversationId:string},context:RequestContext):Promise<ConversationContextDto>{
    const value=await unary<GetProjectConversationContextResponse>(context,(m,o,c)=>client.getProjectConversationContext(input,m,o,c));
    return {sourceWatermark:value.sourceWatermark,archiveRecordCount:value.archiveRecordCount,capacityRecordLimit:value.capacityRecordLimit,recentMessageLimit:value.recentMessageLimit,
      ...(value.summary?{summary:{version:value.summary.version,sourceWatermark:value.summary.sourceWatermark,content:value.summary.content,sourceRecordIds:[...value.summary.sourceRecordIds]}}:{})};
  },
  async listProjectConversationHistory(input:ConversationHistoryInput,context:RequestContext):Promise<ConversationHistoryDto>{
    const value=await unary<ListProjectConversationHistoryResponse>(context,(m,o,c)=>client.listProjectConversationHistory({...input,beforeSequence:input.beforeSequence??"0",snapshotSequence:input.snapshotSequence??"0",query:input.query??"",limit:input.limit??50},m,o,c));
    return {records:value.records.map(r=>({sequence:r.sequence,id:r.id,kind:r.kind,sourceId:r.sourceId,repositoryId:r.repositoryId,role:r.role,content:r.content})),snapshotSequence:value.snapshotSequence,nextBeforeSequence:value.nextBeforeSequence,hasMore:value.hasMore,archiveRecordCount:value.archiveRecordCount};
  },

  async listProjectConversations(projectId:string, context:RequestContext):Promise<ProjectConversationDto[]> {
    const response=await unary<ListProjectConversationsResponse>(context,(m,o,c)=>client.listProjectConversations({projectId},m,o,c));
    return response.conversations.map(mapConversation);
  },
  async getProjectConversation(conversationId:string, context:RequestContext):Promise<ProjectConversationDto> {
    const response=await unary<ProjectConversationResponse>(context,(m,o,c)=>client.getProjectConversation({conversationId},m,o,c));
    return mapConversation(required(response.conversation));
  },
  async createProjectConversation(input:{idempotencyKey:string;projectId:string;title:string;repositoryConnectionIds:string[]},context:RequestContext):Promise<ProjectConversationDto> {
    const response=await unary<ProjectConversationResponse>(context,(m,o,c)=>client.createProjectConversation(input,m,o,c));
    return mapConversation(required(response.conversation));
  },
  async sendProjectMessage(input:{idempotencyKey:string;conversationId:string;content:string},context:RequestContext):Promise<ProjectConversationDto> {
    const response=await unary<ProjectConversationResponse>(context,(m,o,c)=>client.sendProjectMessage(input,m,o,c));
    return mapConversation(required(response.conversation));
  },
  async decideProjectPlan(input:{idempotencyKey:string;conversationId:string;planId:string;version:string;decision:string},context:RequestContext):Promise<ProjectConversationDto> {
    const response=await unary<ProjectConversationResponse>(context,(m,o,c)=>client.decideProjectPlan(input,m,o,c));
    return mapConversation(required(response.conversation));
  },
  async decideProjectProposal(input:{idempotencyKey:string;conversationId:string;proposalId:string;decision:string},context:RequestContext):Promise<ProjectConversationDto> {
    const response=await unary<ProjectConversationResponse>(context,(m,o,c)=>client.decideProjectProposal(input,m,o,c));
    return mapConversation(required(response.conversation));
  },

};}
export type ConversationOperations=ReturnType<typeof conversationOperations>;
