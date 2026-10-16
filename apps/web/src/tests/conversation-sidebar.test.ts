import { beforeEach, expect, it, vi } from "vitest";
const bff = vi.hoisted(() => ({ getWorkspace: vi.fn(), getProject: vi.fn(), getDevelopmentTask: vi.fn(), listProjectConversations: vi.fn() }));
const choices = vi.hoisted(() => ({ workspaceProjects: vi.fn(), projectRepositoryChoices: vi.fn() }));
vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));
vi.mock("$lib/server/project-choices.js", () => choices);
import { loadConversationSidebar } from "$lib/server/conversation-sidebar.js";
const context = { requestId: "sidebar" };
const workspace = { id: "workspace:1", displayName: "Workspace", state: "active", repositoryConnectionIds: ["repository:1"] };
beforeEach(() => {
 vi.resetAllMocks(); bff.getWorkspace.mockResolvedValue(workspace);
 bff.getProject.mockResolvedValue({workspaceId:workspace.id});
 choices.workspaceProjects.mockResolvedValue([{id:"project:1",displayName:"One",state:"active",repositoryConnectionIds:["repository:1","repository:outside"]},{id:"project:2",displayName:"Two",state:"archived",repositoryConnectionIds:["repository:1"]}]);
 choices.projectRepositoryChoices.mockResolvedValue([{id:"repository:1",displayName:"Repo"},{id:"repository:outside",displayName:"Outside"}]);
 bff.listProjectConversations.mockImplementation(async id => [{id:`conversation:${id}`,projectId:id,title:id,createdAt:id==="project:1"?"2026-10-01T00:00:00Z":"2026-10-02T00:00:00Z"}]);
});
it("leaves conversations inactive without a selected workspace",async()=>{
 expect(await loadConversationSidebar({},context)).toBeUndefined();expect(bff.getWorkspace).not.toHaveBeenCalled();
});
it("combines history from workspace projects, including archived history, in date order",async()=>{
 const result=await loadConversationSidebar({projectId:"project:1"},context);
 expect(choices.workspaceProjects).toHaveBeenCalledWith(workspace.id,context);
 expect(result?.conversations.map(c=>c.projectId)).toEqual(["project:2","project:1"]);
 expect(result?.projects[0]?.repositoryConnectionIds).toEqual(["repository:1"]);
 expect(result?.repositories).toEqual([{id:"repository:1",displayName:"Repo"}]);
 expect(result?.incomplete).toBe(false);
});
it("keeps available history and reports partial failures",async()=>{
 bff.listProjectConversations.mockRejectedValueOnce(new Error("Offline"));
 const result=await loadConversationSidebar({workspaceId:workspace.id},context);
 expect(result?.incomplete).toBe(true);expect(result?.conversations).toHaveLength(1);
});
it("resolves the workspace when opening an execution task",async()=>{
 bff.getDevelopmentTask.mockResolvedValue({projectId:"project:1"});
 expect((await loadConversationSidebar({taskId:"task:1"},context))?.workspace.id).toBe(workspace.id);
});

it("keeps conversations associated with their owning project",async()=>{
 bff.listProjectConversations.mockResolvedValue([{id:"conversation:outside",projectId:"project:outside",title:"Outside",createdAt:"2026-10-03T00:00:00Z"}]);
 expect((await loadConversationSidebar({workspaceId:workspace.id},context))?.conversations).toEqual([]);
});
