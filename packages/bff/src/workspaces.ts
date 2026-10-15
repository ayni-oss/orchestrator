import type {Metadata,CallOptions,ClientUnaryCall,ServiceError} from "@grpc/grpc-js";
import {ProjectState,WorkspaceState,type Project,type Workspace,type ProjectServiceClient,type CreateWorkspaceResponse,type GetWorkspaceResponse,type ListWorkspacesResponse,type UpdateWorkspaceResponse,type ArchiveWorkspaceResponse} from "@ayni-orchestrator/grpc-bindings/server";
import type {PageInput,ProjectDto,WorkspaceDto,ProjectAgentDefaultsDto,RequestContext} from "./dto.js";
import {BffMappingError} from "./mapping.js";
export type UnaryInvoker<T> = (
  metadata: Metadata,
  options: Partial<CallOptions>,
  callback: (error: ServiceError | null, response: T) => void,
) => ClientUnaryCall;

type Unary=<T>(context:RequestContext,invoke:UnaryInvoker<T>)=>Promise<T>;
function required<T>(value:T|undefined):T {if(value===undefined)throw new BffMappingError();return value;}
function pageDto(value:{nextPageToken:string;snapshotCursor:string}|undefined){return {...required(value)};}
function mapWorkspace(value:Workspace):WorkspaceDto {
  if(value.repositoryConnectionIds.length<1 || value.repositoryConnectionIds.length>100 || new Set(value.repositoryConnectionIds).size!==value.repositoryConnectionIds.length || ![WorkspaceState.WORKSPACE_STATE_ACTIVE,WorkspaceState.WORKSPACE_STATE_ARCHIVED].includes(value.state))throw new BffMappingError();
  return {plannerIdleTimeoutSeconds:value.plannerIdleTimeoutSeconds||300,id:value.id,displayName:value.displayName,state:value.state===WorkspaceState.WORKSPACE_STATE_ACTIVE?"active":"archived",repositoryConnectionIds:[...value.repositoryConnectionIds],resourceVersion:value.resourceVersion,...(value.agentDefaults?{agentDefaults:{...value.agentDefaults}}:{})};
}

export function mapProject(value: Project): ProjectDto {
  if (value.repositoryConnectionIds.length < 1 || value.repositoryConnectionIds.length > 100 || new Set(value.repositoryConnectionIds).size !== value.repositoryConnectionIds.length || ![ProjectState.PROJECT_STATE_ACTIVE, ProjectState.PROJECT_STATE_ARCHIVED].includes(value.state)) throw new BffMappingError();
  return {workspaceId:value.workspaceId, goal:value.goal, id:value.id, displayName:value.displayName, state:value.state === ProjectState.PROJECT_STATE_ACTIVE ? "active" : "archived", repositoryConnectionIds:[...value.repositoryConnectionIds], resourceVersion:value.resourceVersion, ...(value.agentDefaults ? {agentDefaults:{...value.agentDefaults}} : {})};
}

export function workspaceOperations(client:ProjectServiceClient,unary:Unary){
  return {
  async listWorkspaces(selection:Required<PageInput>, context:RequestContext, activeOnly=false) {
    const response=await unary<ListWorkspacesResponse>(context,(m,o,c)=>client.listWorkspaces({page:selection,...(activeOnly?{state:WorkspaceState.WORKSPACE_STATE_ACTIVE}:{})},m,o,c));
    return {workspaces:response.workspaces.map(mapWorkspace),page:pageDto(response.page)};
  },
  async getWorkspace(workspaceId:string,context:RequestContext):Promise<WorkspaceDto> {
    const response=await unary<GetWorkspaceResponse>(context,(m,o,c)=>client.getWorkspace({workspaceId},m,o,c));
    return mapWorkspace(required(response.workspace));
  },
  async createWorkspace(input:{idempotencyKey:string;displayName:string;repositoryConnectionIds:string[];plannerIdleTimeoutSeconds?:number},context:RequestContext):Promise<WorkspaceDto> {
    const response=await unary<CreateWorkspaceResponse>(context,(m,o,c)=>client.createWorkspace(input,m,o,c));
    return mapWorkspace(required(response.workspace));
  },
  async updateWorkspace(input:{idempotencyKey:string;workspaceId:string;expectedResourceVersion:string;displayName:string;repositoryConnectionIds?:string[];agentDefaults?:ProjectAgentDefaultsDto;plannerIdleTimeoutSeconds?:number},context:RequestContext):Promise<WorkspaceDto> {
    const response=await unary<UpdateWorkspaceResponse>(context,(m,o,c)=>client.updateWorkspace({...input,repositoryConnectionIds:input.repositoryConnectionIds??[]},m,o,c));
    return mapWorkspace(required(response.workspace));
  },
  async archiveWorkspace(input:{idempotencyKey:string;workspaceId:string;expectedResourceVersion:string},context:RequestContext):Promise<WorkspaceDto> {
    const response=await unary<ArchiveWorkspaceResponse>(context,(m,o,c)=>client.archiveWorkspace(input,m,o,c));
    return mapWorkspace(required(response.workspace));
  },

  };
}
export type WorkspaceOperations=ReturnType<typeof workspaceOperations>;
