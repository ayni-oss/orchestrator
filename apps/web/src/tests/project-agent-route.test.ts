import {beforeEach,describe,expect,it,vi} from "vitest";
const metadata={id:"conversation:1",projectId:"project:1",title:"Research",repositoryConnectionIds:["repository:1"],updateCursor:"4",createdAt:"2026-10-13T00:00:00Z",messages:[],runs:[],proposals:[]};
const project={id:"project:1",displayName:"Project",state:"active",repositoryConnectionIds:["repository:1"],resourceVersion:"1"};
const bff=vi.hoisted(()=>({getProject:vi.fn(),getRepositoryConnection:vi.fn(),listRuntimeProfiles:vi.fn(),listDevelopmentTasks:vi.fn(),listProviderModels:vi.fn(),listConnections:vi.fn(),listProjectConversations:vi.fn(),getProjectConversation:vi.fn(),taskProgress:vi.fn(),updateProject:vi.fn(),sendProjectMessage:vi.fn(),decideProjectProposal:vi.fn(),createConnection:vi.fn(),testConnection:vi.fn(),replaceConnectionCredentials:vi.fn()}));
vi.mock("$lib/server/bff.js",()=>({getBff:()=>bff}));
vi.mock("$lib/server/project-choices.js",()=>({projectRepositoryChoices:async()=>[]}));
import {load,actions} from "../routes/projects/[projectId]/+page.server.js";
import {connectionActions} from "$lib/server/connections.js";
function event(fields:Record<string,string>={}){
  const form=new FormData();for(const [key,value] of Object.entries({idempotencyKey:"00000000-0000-4000-8000-000000000001",...fields}))form.set(key,value);
  return {params:{projectId:"project:1"},url:new URL("http://localhost/projects/project:1?conversation=conversation:1"),locals:{requestId:"route-test",requestSignal:new AbortController().signal},request:new Request("http://localhost/projects/project:1",{method:"POST",body:form}),setHeaders:()=>{},depends:()=>{}} as never;
}
describe("project agent operator routes",()=>{
  beforeEach(()=>{
    vi.clearAllMocks();bff.getProject.mockResolvedValue(project);bff.getRepositoryConnection.mockResolvedValue({id:"repository:1"});bff.listRuntimeProfiles.mockResolvedValue({runtimeProfiles:[]});bff.listDevelopmentTasks.mockResolvedValue({developmentTasks:[],page:{nextPageToken:""}});bff.listProviderModels.mockResolvedValue({models:[],provider:"openai"});bff.listConnections.mockResolvedValue([]);bff.listProjectConversations.mockResolvedValue([metadata]);bff.getProjectConversation.mockResolvedValue({...metadata,messages:[{id:"message:1",role:"assistant",content:"Full conversation history",createdAt:metadata.createdAt,runId:"run:1"}],proposals:[{id:"proposal:1",action:"create_task",state:"pending"}]});
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
  it("gives project defaults, messages and approval mutations the bounded 30 second deadline",async()=>{
    await actions.agentDefaults!(event({resourceVersion:"1",projectConnectionId:"connection:1",projectModel:"model",codingConnectionId:"connection:1",codingModel:"model"}));
    await actions.sendMessage!(event({conversationId:"conversation:1",content:"Requested work"}));
    await actions.decideProposal!(event({conversationId:"conversation:1",proposalId:"proposal:1",decision:"approve"}));
    for(const method of [bff.updateProject,bff.sendProjectMessage,bff.decideProjectProposal])expect(method.mock.calls[0]?.[1]).toMatchObject({deadlineMs:30_000});
  });
  it.each(["create","test","replace"])("gives OpenAI %s the bounded 30 second deadline",async operation=>{
    await connectionActions("openai")[operation]!(event({id:"connection:1",displayName:"Models",apiKey:"fixture-key",resourceVersion:"1"}));
    const method=operation==="create"?bff.createConnection:operation==="test"?bff.testConnection:bff.replaceConnectionCredentials;
    expect(method.mock.calls[0]?.[1]).toMatchObject({deadlineMs:30_000});
  });
});
