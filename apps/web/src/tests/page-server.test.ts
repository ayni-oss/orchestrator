import { beforeEach, describe, expect, it, vi } from "vitest";
const bff = vi.hoisted(() => ({
 getWorkspace:vi.fn(),listWorkspaces:vi.fn(),listProjectSummaries:vi.fn(), listProjects:vi.fn(), listRepositoryConnections:vi.fn(), listRuntimeProfiles:vi.fn(), listDevelopmentTasks:vi.fn(), createProject:vi.fn(),
}));
vi.mock("$lib/server/bff.js", () => ({getBff:()=>bff}));
import { load } from "../routes/projects/+page.server.js";
import { actions } from "../routes/workspaces/[workspaceId]/+page.server.js";
import { load as home } from "../routes/+page.server.js";
const locals={requestId:"request",requestSignal:new AbortController().signal};
const page={nextPageToken:"",snapshotCursor:"1"};
beforeEach(()=>{
 vi.clearAllMocks();
 bff.listProjectSummaries.mockResolvedValue({summaries:[],page});
 bff.listWorkspaces.mockResolvedValue({workspaces:[],page});
 bff.getWorkspace.mockResolvedValue({id:"workspace:1",state:"active",repositoryConnectionIds:["repository:1","repository:2"]});
 bff.listProjects.mockResolvedValue({projects:[],page});
 bff.listRepositoryConnections.mockResolvedValue({repositoryConnections:[],page});
 bff.listRuntimeProfiles.mockResolvedValue({runtimeProfiles:[],page});
 bff.listDevelopmentTasks.mockResolvedValue({developmentTasks:[],page});
});
describe("project page server boundary",()=>{
 it("redirects home to workspaces",()=>{expect(()=>home({} as never)).toThrow(expect.objectContaining({status:307,location:"/workspaces"}));});
 it.each([["", "/workspaces"],["?workspace=workspace:1", "/workspaces/workspace%3A1"]])("redirects project index bookmarks %s into workspaces",(query,location)=>{
  expect(()=>load({url:new URL(`http://localhost/projects${query}`)} as never)).toThrow(expect.objectContaining({status:303,location}));
 });
 it("rejects an empty or duplicate membership before dispatch",async()=>{
  for(const repositories of [[],["repository:1","repository:1"]]) {
   const body=new FormData();body.set("idempotencyKey","00000000-0000-4000-8000-000000000099");body.set("displayName","Payments");body.set("workspaceId","workspace:1");body.set("goal","Ship payments");
   for(const id of repositories) body.append("repositoryConnectionIds",id);
   expect(await actions.create!({locals,params:{workspaceId:"workspace:1"},request:new Request("http://localhost/projects?/create",{method:"POST",body})} as never)).toMatchObject({status:400});
  }
  expect(bff.createProject).not.toHaveBeenCalled();
 });
 it("dispatches the complete selected membership",async()=>{
  bff.createProject.mockResolvedValueOnce({id:"project:1"});
  const body=new FormData();body.set("idempotencyKey","00000000-0000-4000-8000-000000000099");body.set("displayName","Payments");body.set("workspaceId","workspace:1");body.set("goal","Ship payments");body.append("repositoryConnectionIds","repository:1");body.append("repositoryConnectionIds","repository:2");
  await expect(actions.create!({locals,params:{workspaceId:"workspace:1"},request:new Request("http://localhost/projects?/create",{method:"POST",body})} as never)).rejects.toMatchObject({status:303,location:"/projects/project%3A1"});
  expect(bff.createProject).toHaveBeenCalledWith(expect.objectContaining({repositoryConnectionIds:["repository:1","repository:2"]}),expect.anything());
 });
});
