import {BffError} from "@ayni-orchestrator/bff/server";
import {beforeEach,describe,expect,it,vi} from "vitest";
const metadata={id:"conversation:1",projectId:"project:1",title:"Research",repositoryConnectionIds:["repository:1"],updateCursor:"4",createdAt:"2026-10-13T00:00:00Z",messages:[],runs:[],proposals:[],plan:{id:"plan:1",versions:[{version:"1",state:"superseded",proposalIds:["proposal:1"]},{version:"2",state:"pending",proposalIds:["proposal:2"]}]}};
const workspace={id:"workspace:1",displayName:"Workspace",state:"active",repositoryConnectionIds:["repository:1"],resourceVersion:"1",agentDefaults:{projectConnectionId:"workspace-connection",projectModel:"workspace-model",codingConnectionId:"workspace-connection",codingModel:"workspace-model"}};
const project={workspaceId:"workspace:1",goal:"Complete work",id:"project:1",displayName:"Project",state:"active",repositoryConnectionIds:["repository:1"],resourceVersion:"1"};
const bff=vi.hoisted(()=>({createProjectConversation:vi.fn(),getProject:vi.fn(),getWorkspace:vi.fn(),getRepositoryConnection:vi.fn(),listRuntimeProfiles:vi.fn(),listDevelopmentTasks:vi.fn(),listProviderModels:vi.fn(),listConnections:vi.fn(),listProjectConversations:vi.fn(),getProjectConversation:vi.fn(),updateProjectConversationSettings:vi.fn(),cancelProjectAgentRun:vi.fn(),taskProgress:vi.fn(),updateProject:vi.fn(),sendProjectMessage:vi.fn(),decideProjectProposal:vi.fn(),decideProjectPlan:vi.fn(),createConnection:vi.fn(),testConnection:vi.fn(),replaceConnectionCredentials:vi.fn()}));
vi.mock("$lib/server/bff.js",()=>({getBff:()=>bff}));
vi.mock("$lib/server/project-choices.js",()=>({projectRepositoryChoices:async()=>[],workspaceChoices:async()=>[workspace],workspaceProjects:async()=>[project]}));
import {load,actions} from "../routes/projects/[projectId]/+page.server.js";
import {connectionActions} from "$lib/server/connections.js";
function event(fields:Record<string,string>={},url="http://localhost/projects/project:1?conversation=conversation:1"){
  const form=new FormData();for(const [key,value] of Object.entries({idempotencyKey:"00000000-0000-4000-8000-000000000001",...fields}))form.set(key,value);
  return {params:{projectId:"project:1"},url:new URL(url),locals:{requestId:"route-test",requestSignal:new AbortController().signal},request:new Request("http://localhost/projects/project:1",{method:"POST",body:form}),setHeaders:()=>{},depends:()=>{}} as never;
}
describe("project agent operator routes",()=>{
  beforeEach(()=>{
    vi.clearAllMocks();bff.createProjectConversation.mockResolvedValue(metadata);bff.getProject.mockResolvedValue(project);bff.getWorkspace.mockResolvedValue(workspace);bff.getRepositoryConnection.mockResolvedValue({id:"repository:1"});bff.listRuntimeProfiles.mockResolvedValue({runtimeProfiles:[]});bff.listDevelopmentTasks.mockResolvedValue({developmentTasks:[],page:{nextPageToken:""}});bff.listProviderModels.mockResolvedValue({models:[],provider:"openai"});bff.listConnections.mockResolvedValue([]);bff.listProjectConversations.mockResolvedValue([metadata]);bff.getProjectConversation.mockResolvedValue({...metadata,runs:[{id:"run:1",state:"running"}],messages:[{id:"message:1",role:"assistant",content:"Full conversation history",createdAt:metadata.createdAt,runId:"run:1"}],proposals:[{id:"proposal:1",action:"create_task",state:"pending"}]});
  });
  it("opens a blank composer when no conversation is selected, retaining history",async()=>{
    expect(await load(event({},"http://localhost/projects/project:1?view=overview&new=fresh"))).toMatchObject({conversation:undefined,conversations:[metadata]});
    expect(bff.getProjectConversation).not.toHaveBeenCalled();
  });
  it("derives a 30-character title and sends the full first message before redirecting",async()=>{
    const content="Research  the next API improvement\r\nwith backwards compatibility";
    await expect(actions.createConversation!(event({content}))).rejects.toMatchObject({status:303,location:"/projects/project%3A1?conversation=conversation%3A1"});
    expect(bff.createProjectConversation).toHaveBeenCalledWith(expect.objectContaining({title:"Research the next API improvem",repositoryConnectionIds:["repository:1"]}),expect.anything());
    expect(bff.sendProjectMessage).toHaveBeenCalledWith(expect.objectContaining({conversationId:metadata.id,content,idempotencyKey:"00000000-0000-4000-8000-000000000001-message"}),expect.anything());
  });
  it("keeps Unicode characters intact in generated conversation titles",async()=>{
    const content="🚀".repeat(35);
    await expect(actions.createConversation!(event({content}))).rejects.toMatchObject({status:303});
    expect(bff.createProjectConversation).toHaveBeenCalledWith(expect.objectContaining({title:"🚀".repeat(30)}),expect.anything());
  });
  it.each(["", " ", "x".repeat(16385)])("rejects empty or oversized first messages before creating a conversation",async content=>{
    expect(await actions.createConversation!(event({content}))).toMatchObject({status:400});
    expect(bff.createProjectConversation).not.toHaveBeenCalled();
    expect(bff.sendProjectMessage).not.toHaveBeenCalled();
  });
  it.each(["repository:1","repository:other"])("inherits every project repository regardless of submitted selection %s",async repositoryConnectionIds=>{
    bff.getProject.mockResolvedValueOnce({...project,repositoryConnectionIds:["repository:1","repository:2"]});
    await expect(actions.createConversation!(event({content:"Research changes",repositoryConnectionIds}))).rejects.toMatchObject({status:303});
    expect(bff.createProjectConversation).toHaveBeenCalledWith(expect.objectContaining({repositoryConnectionIds:["repository:1","repository:2"]}),expect.anything());
  });
  it("requires project repositories before starting a conversation",async()=>{
    bff.getProject.mockResolvedValueOnce({...project,repositoryConnectionIds:[]});
    expect(await actions.createConversation!(event({content:"Research changes"}))).toMatchObject({status:400});
    expect(bff.createProjectConversation).not.toHaveBeenCalled();
    expect(bff.sendProjectMessage).not.toHaveBeenCalled();
  });
  it("reuses creation and message idempotency keys after a first-message failure",async()=>{
    bff.sendProjectMessage.mockRejectedValueOnce(new BffError({status:503,reason:"unavailable",message:"Try again",retryable:true}));
    const fields={content:"Retry research"};
    expect(await actions.createConversation!(event(fields))).toMatchObject({status:503});
    await expect(actions.createConversation!(event(fields))).rejects.toMatchObject({status:303});
    for(const method of [bff.createProjectConversation,bff.sendProjectMessage])expect(method.mock.calls[0]?.[0]).toEqual(method.mock.calls[1]?.[0]);
  });
  it("loads selected conversation details separately from metadata-only history",async()=>{
    const result=await load(event()) as {conversation?:{messages:Array<{content:string}>}};
    expect(bff.getProjectConversation).toHaveBeenCalledWith("conversation:1",expect.objectContaining({requestId:"route-test"}));
    expect(result.conversation?.messages[0]?.content).toBe("Full conversation history");
  });
  it("does not reflect details belonging to another project",async()=>{
    bff.getProjectConversation.mockResolvedValueOnce({...metadata,projectId:"project:other"});
    const result=await load(event()) as {conversation?:unknown;conversationError?:{status:number}};
    expect(result.conversation).toBeUndefined();expect(result.conversationError?.status).toBe(404);
  });
  it("reports an unavailable selected conversation without silently switching records",async()=>{
    bff.listProjectConversations.mockResolvedValueOnce([]);
    expect(await load(event())).toMatchObject({conversation:undefined,conversationError:{status:404}});
    expect(bff.getProjectConversation).not.toHaveBeenCalled();
  });
  it("retains linked task results when the task list is on another page",async()=>{
    bff.getProjectConversation.mockResolvedValueOnce({...metadata,proposals:[{id:"proposal:1",taskId:"task:older"}]});
    bff.taskProgress.mockResolvedValueOnce({attempts:[{id:"attempt:older"}]});
    expect(await load(event())).toMatchObject({taskProgress:{"task:older":{attempts:[{id:"attempt:older"}]}}});
  });
  it("gives project defaults, messages and approval mutations the bounded 30 second deadline",async()=>{
    await actions.agentDefaults!(event({resourceVersion:"1",projectConnectionId:"connection:1",projectModel:"model",codingConnectionId:"connection:1",codingModel:"model"}));
    await actions.sendMessage!(event({conversationId:"conversation:1",content:"Requested work"}));
    await actions.decideProposal!(event({conversationId:"conversation:1",proposalId:"proposal:1",decision:"approve"}));
    for(const method of [bff.updateProject,bff.sendProjectMessage,bff.decideProjectProposal])expect(method.mock.calls[0]?.[1]).toMatchObject({deadlineMs:30_000});
  });
  it("saves explicit planner presets and inheritance with scope and submitted cursor",async()=>{
    await actions.plannerSettings!(event({conversationId:"conversation:1",plannerIdleTimeoutSeconds:"86400",expectedUpdateCursor:"4"}));
    expect(bff.updateProjectConversationSettings).toHaveBeenCalledWith(expect.objectContaining({projectId:"project:1",conversationId:"conversation:1",expectedUpdateCursor:"4",plannerIdleTimeoutSeconds:86400,inheritWorkspaceTimeout:false}),expect.objectContaining({deadlineMs:30000}));
    await actions.plannerSettings!(event({conversationId:"conversation:1",plannerIdleTimeoutSeconds:"inherit",expectedUpdateCursor:"4"}));
    expect(bff.updateProjectConversationSettings.mock.calls[1]?.[0]).toMatchObject({inheritWorkspaceTimeout:true});expect(bff.updateProjectConversationSettings.mock.calls[1]?.[0]).not.toHaveProperty("plannerIdleTimeoutSeconds");
  });
  it.each(["301","0","NaN","3600"])("rejects unsupported planner timeout %s",async timeout=>{
    expect(await actions.plannerSettings!(event({conversationId:"conversation:1",plannerIdleTimeoutSeconds:timeout,expectedUpdateCursor:"4"}))).toMatchObject({status:400});expect(bff.updateProjectConversationSettings).not.toHaveBeenCalled();
  });
  it("preserves stale settings errors and cancels only turns within the selected project conversation",async()=>{
    bff.updateProjectConversationSettings.mockRejectedValueOnce(new BffError({status:409,reason:"stale_version",message:"Reload conversation settings",retryable:false}));
    expect(await actions.plannerSettings!(event({conversationId:"conversation:1",plannerIdleTimeoutSeconds:"300",expectedUpdateCursor:"3"}))).toMatchObject({status:409});
    expect(await actions.cancelPlannerTurn!(event({conversationId:"conversation:1",runId:"run:other"}))).toMatchObject({status:400});expect(bff.cancelProjectAgentRun).not.toHaveBeenCalled();
    await actions.cancelPlannerTurn!(event({conversationId:"conversation:1",runId:"run:1"}));expect(bff.cancelProjectAgentRun).toHaveBeenCalledWith(expect.objectContaining({projectId:"project:1",conversationId:"conversation:1",runId:"run:1"}),expect.objectContaining({deadlineMs:30000}));
    bff.getProjectConversation.mockResolvedValueOnce({...metadata,projectId:"project:other"});expect(await actions.cancelPlannerTurn!(event({conversationId:"conversation:1",runId:"run:1"}))).toMatchObject({status:400});expect(bff.cancelProjectAgentRun).toHaveBeenCalledTimes(1);
  });
  it.each(["queued","running"])("cancels a retained %s turn without requiring a live session or active workspace",async state=>{
    bff.getWorkspace.mockResolvedValue({...workspace,state:"archived"});
    bff.getProjectConversation.mockResolvedValueOnce({...metadata,runs:[{id:"run:1",state}]});
    expect(await actions.cancelPlannerTurn!(event({conversationId:"conversation:1",runId:"run:1"}))).toMatchObject({success:expect.stringContaining("cancellation requested")});
    expect(bff.cancelProjectAgentRun).toHaveBeenCalledWith(expect.objectContaining({runId:"run:1",projectId:"project:1"}),expect.anything());
  });
  it("binds decisions to the submitted exact plan version instead of silently selecting the newest",async()=>{
    await actions.decidePlan!(event({conversationId:"conversation:1",planId:"plan:1",version:"1",decision:"approve"}));
    expect(bff.decideProjectPlan).toHaveBeenCalledWith(expect.objectContaining({conversationId:"conversation:1",planId:"plan:1",version:"1",decision:"approve"}),expect.objectContaining({deadlineMs:30_000}));
  });
  it("rejects unknown plans and malformed versions before mutation",async()=>{
    for(const fields of [{planId:"plan:other",version:"2"},{planId:"plan:1",version:"0"},{planId:"plan:1",version:"3"}]){
      const result=await actions.decidePlan!(event({conversationId:"conversation:1",decision:"approve",...fields}));
      expect(result).toMatchObject({status:400});
    }
    expect(bff.decideProjectPlan).not.toHaveBeenCalled();
  });
  it("makes stale source and runtime approval failures actionable",async()=>{
    bff.decideProjectPlan.mockRejectedValueOnce(new BffError({status:412,reason:"failed_precondition",message:"current inputs unavailable",retryable:false}));
    const result=await actions.decidePlan!(event({conversationId:"conversation:1",planId:"plan:1",version:"2",decision:"approve"}));
    expect(result).toMatchObject({status:412,data:{error:{message:expect.stringContaining("request a revised plan")}}});
  });
  it.each(["create","test","replace"])("gives OpenAI %s the bounded 30 second deadline",async operation=>{
    await connectionActions("openai")[operation]!(event({id:"connection:1",displayName:"Models",apiKey:"fixture-key",resourceVersion:"1"}));
    const method=operation==="create"?bff.createConnection:operation==="test"?bff.testConnection:bff.replaceConnectionCredentials;
    expect(method.mock.calls[0]?.[1]).toMatchObject({deadlineMs:30_000});
  });
});
