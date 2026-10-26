import {beforeEach,describe,expect,it,vi} from "vitest";
const bff=vi.hoisted(()=>({getProject:vi.fn(),getRuntimeProfile:vi.fn(),createDevelopmentTask:vi.fn(),getWorkspace:vi.fn(),listDevelopmentTasks:vi.fn(),getAgentCapabilities:vi.fn(),getProjectConfiguration:vi.fn(),listConnections:vi.fn(),listProviderModels:vi.fn()}));
vi.mock("$lib/server/bff.js",()=>({getBff:()=>bff}));
import {load} from "../routes/tasks/+page.server.js";
import {submitDirectTask} from "$lib/server/tasks.js";
const project={id:"project:1",workspaceId:"workspace:1",state:"active",repositoryConnectionIds:["repository:1"]};
const workspace={id:"workspace:1",state:"active",repositoryConnectionIds:["repository:1"]};
const page={nextPageToken:"",snapshotCursor:"1"};
beforeEach(()=>{vi.resetAllMocks();bff.getProject.mockResolvedValue(project);bff.getWorkspace.mockResolvedValue(workspace);bff.getAgentCapabilities.mockResolvedValue({});bff.getProjectConfiguration.mockResolvedValue({project});bff.listConnections.mockResolvedValue([{id:"coding",kind:"openai",enabled:true,status:"ready"}]);bff.listProviderModels.mockResolvedValue({models:[{id:"coding"}],page});bff.listDevelopmentTasks.mockResolvedValue({developmentTasks:[],page});});
const locals=()=>({requestId:"task-workspace",requestSignal:new AbortController().signal});
describe("task history and direct submission",()=>{
 it("routes the task index to operations",()=>{expect(()=>load({url:new URL("http://localhost/tasks")} as never)).toThrow(expect.objectContaining({status:303,location:"/operations"}));});
 it("routes project task bookmarks into the project Tasks tab",()=>{expect(()=>load({url:new URL("http://localhost/tasks?project=project:1")} as never)).toThrow(expect.objectContaining({status:303,location:"/projects/project%3A1?view=tasks"}));});
 it.each(["archived","removed"])("rejects direct submission for %s workspace access",async condition=>{
   bff.getWorkspace.mockResolvedValueOnce(condition==="archived"?{...workspace,state:"archived"}:{...workspace,repositoryConnectionIds:[]});
   const form=new FormData();for(const [key,value] of Object.entries({idempotencyKey:"00000000-0000-4000-8000-000000000001",projectId:project.id,repositoryConnectionId:"repository:1",runtimeProfileId:"profile:1",connectionId:"coding",provider:"openai",model:"coding",request:"Ship work",acceptanceCriteria:"Behavior works"}))form.set(key,value);
   expect(await submitDirectTask({locals:locals(),params:{projectId:project.id},request:new Request("http://localhost/projects/project:1",{method:"POST",body:form})} as never)).toMatchObject({status:409});
   expect(bff.getRuntimeProfile).not.toHaveBeenCalled();expect(bff.createDevelopmentTask).not.toHaveBeenCalled();
 });
});
