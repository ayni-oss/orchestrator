import { beforeEach, describe, expect, it, vi } from "vitest";
const bff = vi.hoisted(() => ({
 listProjectSummaries:vi.fn(), listProjects:vi.fn(), listRepositoryConnections:vi.fn(), listRuntimeProfiles:vi.fn(), listDevelopmentTasks:vi.fn(), createProject:vi.fn(),
}));
vi.mock("$lib/server/bff.js", () => ({getBff:()=>bff}));
import { load, actions } from "../routes/projects/+page.server.js";
import { load as home } from "../routes/+page.server.js";
const locals={requestId:"request",requestSignal:new AbortController().signal};
const page={nextPageToken:"",snapshotCursor:"1"};
beforeEach(()=>{
 vi.clearAllMocks();
 bff.listProjectSummaries.mockResolvedValue({summaries:[],page});
 bff.listProjects.mockResolvedValue({projects:[],page});
 bff.listRepositoryConnections.mockResolvedValue({repositoryConnections:[],page});
 bff.listRuntimeProfiles.mockResolvedValue({runtimeProfiles:[],page});
 bff.listDevelopmentTasks.mockResolvedValue({developmentTasks:[],page});
});
describe("project page server boundary",()=>{
 it("redirects home to projects",()=>{expect(()=>home({} as never)).toThrow(expect.objectContaining({status:307,location:"/projects"}));});
 it("loads project state through the BFF with no caching",async()=>{
  const setHeaders=vi.fn();
  const result=await load({locals,depends:vi.fn(),setHeaders,url:new URL("http://localhost/projects")} as never);
  expect(result).toMatchObject({summaries:[],repositories:[],available:true});
  expect(setHeaders).toHaveBeenCalledWith({"cache-control":"no-store"});
 });
 it("retains partial results and sanitizes backend failures",async()=>{
  bff.listProjectSummaries.mockRejectedValueOnce(new Error("private backend detail"));
  const result=await load({locals,depends:vi.fn(),setHeaders:vi.fn(),url:new URL("http://localhost/projects")} as never);
  expect(result).toMatchObject({available:false,error:{reason:"internal"}});
  expect(JSON.stringify(result)).not.toContain("private backend detail");
 });
 it("uses complete server summaries without listing global profiles or tasks",async()=>{
  bff.listProjectSummaries.mockResolvedValueOnce({summaries:[{project:{id:"project:1"},tasksComplete:true,latestTask:{id:"task:101"}}],page:{...page,nextPageToken:"next"}});
  expect(await load({locals,depends:vi.fn(),setHeaders:vi.fn(),url:new URL("http://localhost/projects")} as never)).toMatchObject({summaries:[{latestTask:{id:"task:101"}}],nextPageToken:"next"});
  expect(bff.listRuntimeProfiles).not.toHaveBeenCalled();
  expect(bff.listDevelopmentTasks).not.toHaveBeenCalled();
 });
 it("rejects an empty or duplicate membership before dispatch",async()=>{
  for(const repositories of [[],["repository:1","repository:1"]]) {
   const body=new FormData();body.set("idempotencyKey","00000000-0000-4000-8000-000000000099");body.set("displayName","Payments");
   for(const id of repositories) body.append("repositoryConnectionIds",id);
   expect(await actions.create!({locals,request:new Request("http://localhost/projects?/create",{method:"POST",body})} as never)).toMatchObject({status:400});
  }
  expect(bff.createProject).not.toHaveBeenCalled();
 });
 it("dispatches the complete selected membership",async()=>{
  bff.createProject.mockResolvedValueOnce({id:"project:1"});
  const body=new FormData();body.set("idempotencyKey","00000000-0000-4000-8000-000000000099");body.set("displayName","Payments");body.append("repositoryConnectionIds","repository:1");body.append("repositoryConnectionIds","repository:2");
  await expect(actions.create!({locals,request:new Request("http://localhost/projects?/create",{method:"POST",body})} as never)).rejects.toMatchObject({status:303,location:"/projects/project%3A1"});
  expect(bff.createProject).toHaveBeenCalledWith(expect.objectContaining({repositoryConnectionIds:["repository:1","repository:2"]}),expect.anything());
 });
});
