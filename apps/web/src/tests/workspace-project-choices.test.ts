import {beforeEach,describe,expect,it,vi} from "vitest";
const bff=vi.hoisted(()=>({listProjects:vi.fn()}));
vi.mock("$lib/server/bff.js",()=>({getBff:()=>bff}));
import {workspaceProjects} from "$lib/server/project-choices.js";
const context={requestId:"workspace-projects"};
beforeEach(()=>vi.clearAllMocks());
describe("workspace project discovery",()=>{
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
