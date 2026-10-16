import {beforeEach,describe,expect,it,vi} from "vitest";
const bff=vi.hoisted(()=>({listProjects:vi.fn(),listRepositorySummaries:vi.fn()}));
vi.mock("$lib/server/bff.js",()=>({getBff:()=>bff}));
import {workspaceProjects,workspaceProjectCounts,repositorySummaryChoices} from "$lib/server/project-choices.js";
const context={requestId:"workspace-projects"};
beforeEach(()=>vi.clearAllMocks());
describe("workspace project discovery",()=>{
  it("counts every workspace across project pages",async()=>{
    bff.listProjects.mockResolvedValueOnce({projects:[{workspaceId:"workspace:1"},{workspaceId:"workspace:other"}],page:{nextPageToken:"next"}}).mockResolvedValueOnce({projects:[{workspaceId:"workspace:1"}],page:{nextPageToken:""}});
    expect(await workspaceProjectCounts(context)).toEqual({"workspace:1":2,"workspace:other":1});
  });
  it("rejects incomplete project counts",async()=>{
    bff.listProjects.mockResolvedValue({projects:[],page:{nextPageToken:"same"}});
    await expect(workspaceProjectCounts(context)).rejects.toThrow("could not be loaded completely");
  });
  it("includes workspace projects after pages owned entirely by other workspaces",async()=>{
    bff.listProjects.mockResolvedValueOnce({projects:[{id:"other",workspaceId:"workspace:other"}],page:{nextPageToken:"next"}}).mockResolvedValueOnce({projects:[{id:"child",workspaceId:"workspace:1"}],page:{nextPageToken:""}});
    expect(await workspaceProjects("workspace:1",context)).toEqual([{id:"child",workspaceId:"workspace:1"}]);
    expect(bff.listProjects).toHaveBeenLastCalledWith({pageSize:100,pageToken:"next"},context);
  });
  it("fails instead of reporting a partial project list for looping pagination",async()=>{
    bff.listProjects.mockResolvedValue({projects:[],page:{nextPageToken:"same"}});
    await expect(workspaceProjects("workspace:1",context)).rejects.toThrow("could not be loaded completely");
  });
});

describe("repository summary discovery",()=>{
  it("loads repository cards and membership choices across every page",async()=>{
    bff.listRepositorySummaries.mockResolvedValueOnce({summaries:[{repository:{id:"first"}}],page:{nextPageToken:"next"}}).mockResolvedValueOnce({summaries:[{repository:{id:"last"}}],page:{nextPageToken:""}});
    expect(await repositorySummaryChoices(context)).toEqual([{repository:{id:"first"}},{repository:{id:"last"}}]);
    expect(bff.listRepositorySummaries).toHaveBeenLastCalledWith({pageSize:100,pageToken:"next"},context);
  });
});
