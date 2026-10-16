import {beforeEach,describe,expect,it,vi} from "vitest";
const bff=vi.hoisted(()=>({getWorkspace:vi.fn(),listProjects:vi.fn(),listConnections:vi.fn(),listProviderModels:vi.fn(),updateWorkspace:vi.fn(),archiveWorkspace:vi.fn(),createProject:vi.fn(),listWorkspaces:vi.fn()}));
vi.mock("$lib/server/bff.js",()=>({getBff:()=>bff}));
vi.mock("$lib/server/project-choices.js",()=>({projectRepositoryChoices:async()=>[{id:"repository:1"},{id:"repository:outside"}],workspaceChoices:async()=>[],workspaceProjects:async()=>[{id:"project:1",workspaceId:"workspace:1"}]}));
import {load,actions} from "../routes/workspaces/[workspaceId]/+page.server.js";
const workspace={id:"workspace:1",displayName:"Shared",state:"active",repositoryConnectionIds:["repository:1"],resourceVersion:"3"};
function event(fields:Record<string,string|string[]>={}){
  const form=new FormData();for(const [key,value] of Object.entries({idempotencyKey:"00000000-0000-4000-8000-000000000001",resourceVersion:"3",...fields}))for(const item of Array.isArray(value)?value:[value])form.append(key,item);
  return {params:{workspaceId:workspace.id},url:new URL("http://localhost/workspaces/workspace:1"),locals:{requestId:"workspace-route",requestSignal:new AbortController().signal},request:new Request("http://localhost/workspaces/workspace:1",{method:"POST",body:form}),setHeaders:vi.fn(),depends:vi.fn()} as never;
}
beforeEach(()=>{vi.clearAllMocks();bff.getWorkspace.mockResolvedValue(workspace);bff.listConnections.mockResolvedValue([]);bff.listProjects.mockResolvedValue({projects:[{id:"project:1",workspaceId:workspace.id},{id:"project:other",workspaceId:"workspace:other"}],page:{nextPageToken:"next"}});});
describe("workspace operator routes",()=>{
  it("keeps existing project links scoped to their workspace",async()=>{
    expect(await load(event())).toMatchObject({workspace,projects:[{id:"project:1"}],nextPageToken:""});
  });
  it("updates repository membership with optimistic concurrency",async()=>{
    await actions.update!(event({displayName:"Renamed",repositoryConnectionIds:["repository:1","repository:outside"]}));
    expect(bff.updateWorkspace).toHaveBeenCalledWith(expect.objectContaining({workspaceId:workspace.id,displayName:"Renamed",expectedResourceVersion:"3",repositoryConnectionIds:["repository:1","repository:outside"]}),expect.objectContaining({deadlineMs:30000}));
  });
  it("sets only the supported workspace timeout presets",async()=>{
    await actions.update!(event({displayName:"Shared",repositoryConnectionIds:["repository:1"],plannerIdleTimeoutSeconds:"86400"}));expect(bff.updateWorkspace).toHaveBeenCalledWith(expect.objectContaining({plannerIdleTimeoutSeconds:86400}),expect.anything());
    bff.updateWorkspace.mockClear();expect(await actions.update!(event({displayName:"Shared",repositoryConnectionIds:["repository:1"],plannerIdleTimeoutSeconds:"3600"}))).toMatchObject({status:400});expect(bff.updateWorkspace).not.toHaveBeenCalled();
  });
  it("rejects empty membership before dispatch",async()=>{
    expect(await actions.update!(event({displayName:"Shared"}))).toMatchObject({status:400});expect(bff.updateWorkspace).not.toHaveBeenCalled();
  });
  it("archives the workspace without editing any project",async()=>{
    expect(await actions.archive!(event())).toMatchObject({success:expect.stringContaining("history is retained")});
    expect(bff.archiveWorkspace).toHaveBeenCalledWith(expect.objectContaining({workspaceId:workspace.id,expectedResourceVersion:"3"}),expect.anything());expect(bff.updateWorkspace).not.toHaveBeenCalled();
  });
  it.each(["outside","archived","mismatched"])("rejects project creation for %s workspace resources",async kind=>{
    if(kind==="archived")bff.getWorkspace.mockResolvedValueOnce({...workspace,state:"archived"});
    expect(await actions.create!(event({displayName:"Project",workspaceId:kind==="mismatched"?"workspace:other":workspace.id,goal:"Ship work",repositoryConnectionIds:[kind==="outside"?"repository:outside":"repository:1"]}))).toMatchObject({status:400});expect(bff.createProject).not.toHaveBeenCalled();
  });
});
