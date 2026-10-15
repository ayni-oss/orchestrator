import {Server,ServerCredentials} from "@grpc/grpc-js";
import {ProjectServiceService,ProjectState,WorkspaceState,type Workspace,type Project,type ProjectServiceServer} from "@ayni-orchestrator/grpc-bindings/server";
import {afterEach,beforeEach,describe,expect,it} from "vitest";
import {OrchestratorBff} from "../src/index.js";
const defaults={projectConnectionId:"connection:1",projectModel:"research",codingConnectionId:"connection:1",codingModel:"coding"};
describe("workspace and project transport",()=>{
  let server:Server,bff:OrchestratorBff;
  let workspace:Workspace,project:Project;
  let received:Record<string,unknown>;
  beforeEach(async()=>{
    received={};workspace={plannerIdleTimeoutSeconds:300,id:"workspace:1",displayName:"Shared",state:WorkspaceState.WORKSPACE_STATE_ACTIVE,repositoryConnectionIds:["repository:1"],resourceVersion:"1",agentDefaults:defaults};
    project={id:"project:1",workspaceId:workspace.id,goal:"Ship feature",displayName:"Feature",state:ProjectState.PROJECT_STATE_ACTIVE,repositoryConnectionIds:["repository:1"],resourceVersion:"1"};
    server=new Server();server.addService(ProjectServiceService,{
      createWorkspace(call,callback){received={...call.request};callback(null,{workspace});},
      listWorkspaces(_call,callback){callback(null,{workspaces:[workspace],page:{nextPageToken:"next",snapshotCursor:"4"}});},
      getWorkspace(_call,callback){callback(null,{workspace});},
      updateWorkspace(call,callback){received={...call.request};workspace={...workspace,...(call.request.plannerIdleTimeoutSeconds===undefined?{}:{plannerIdleTimeoutSeconds:call.request.plannerIdleTimeoutSeconds}),displayName:call.request.displayName,...(call.request.agentDefaults?{agentDefaults:call.request.agentDefaults}:{})};callback(null,{workspace});},
      archiveWorkspace(call,callback){received={...call.request};workspace={...workspace,state:WorkspaceState.WORKSPACE_STATE_ARCHIVED};callback(null,{workspace});},
      createProject(call,callback){received={...call.request};callback(null,{project});},
      updateProject(call,callback){received={...call.request};project={...project,goal:call.request.goal??project.goal};callback(null,{project});},
    } satisfies Partial<ProjectServiceServer>);
    const port=await new Promise<number>((resolve,reject)=>server.bindAsync("127.0.0.1:0",ServerCredentials.createInsecure(),(error,port)=>error?reject(error):resolve(port)));
    bff=new OrchestratorBff({address:`127.0.0.1:${port}`});
  });
  afterEach(()=>{bff?.close();server?.forceShutdown();});
  const ctx={requestId:"workspace-test"};
  it("preserves workspace membership, defaults, pagination and archive state",async()=>{
    expect(await bff.createWorkspace({idempotencyKey:"create",displayName:"Shared",repositoryConnectionIds:["repository:1"]},ctx)).toMatchObject({id:"workspace:1",agentDefaults:defaults});
    expect(received).toMatchObject({repositoryConnectionIds:["repository:1"]});
    expect(await bff.listWorkspaces({pageSize:1},ctx)).toMatchObject({workspaces:[{id:"workspace:1"}],page:{nextPageToken:"next"}});
    expect(await bff.getWorkspace("workspace:1",ctx)).toMatchObject({state:"active"});
    await bff.updateWorkspace({idempotencyKey:"update",workspaceId:"workspace:1",expectedResourceVersion:"1",displayName:"Renamed",agentDefaults:defaults},ctx);
    expect(received).toMatchObject({expectedResourceVersion:"1",repositoryConnectionIds:[],agentDefaults:defaults});
    expect(await bff.archiveWorkspace({idempotencyKey:"archive",workspaceId:"workspace:1",expectedResourceVersion:"1"},ctx)).toMatchObject({state:"archived",displayName:"Renamed",repositoryConnectionIds:["repository:1"]});
  });
  it("maps planner timeout presets and preserves omitted settings during model updates",async()=>{
    expect(await bff.getWorkspace("workspace:1",ctx)).toMatchObject({plannerIdleTimeoutSeconds:300});
    await bff.updateWorkspace({idempotencyKey:"timeout",workspaceId:"workspace:1",expectedResourceVersion:"1",displayName:"Shared",plannerIdleTimeoutSeconds:86400},ctx);
    expect(received).toMatchObject({plannerIdleTimeoutSeconds:86400});
    expect(await bff.getWorkspace("workspace:1",ctx)).toMatchObject({plannerIdleTimeoutSeconds:86400});
    await bff.updateWorkspace({idempotencyKey:"models",workspaceId:"workspace:1",expectedResourceVersion:"1",displayName:"Shared",agentDefaults:defaults},ctx);
    expect(received.plannerIdleTimeoutSeconds).toBeUndefined();
    expect(await bff.getWorkspace("workspace:1",ctx)).toMatchObject({plannerIdleTimeoutSeconds:86400});
  });
  it("retains project identity, workspace and goal across creation and updates",async()=>{
    expect(await bff.createProject({idempotencyKey:"create",displayName:"Feature",workspaceId:"workspace:1",goal:"Ship feature",repositoryConnectionIds:["repository:1"]},ctx)).toMatchObject({id:"project:1",workspaceId:"workspace:1",goal:"Ship feature"});
    expect(received).toMatchObject({workspaceId:"workspace:1",goal:"Ship feature"});
    expect(await bff.updateProject({idempotencyKey:"update",projectId:"project:1",expectedResourceVersion:"1",displayName:"Feature",goal:"Refine feature"},ctx)).toMatchObject({id:"project:1",workspaceId:"workspace:1",goal:"Refine feature"});
  });
  it("rejects invalid workspace lifecycle and duplicate membership",async()=>{
    workspace={...workspace,state:WorkspaceState.WORKSPACE_STATE_UNSPECIFIED};
    await expect(bff.getWorkspace("workspace:1",ctx)).rejects.toThrow();
    workspace={...workspace,state:WorkspaceState.WORKSPACE_STATE_ACTIVE,repositoryConnectionIds:["repository:1","repository:1"]};
    await expect(bff.getWorkspace("workspace:1",ctx)).rejects.toThrow();
  });
});
