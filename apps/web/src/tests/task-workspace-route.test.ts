import {beforeEach,describe,expect,it,vi} from "vitest";
const bff=vi.hoisted(()=>({getProject:vi.fn(),getRuntimeProfile:vi.fn(),createDevelopmentTask:vi.fn(),listProjects:vi.fn(),getWorkspace:vi.fn(),listDevelopmentTasks:vi.fn(),listRuntimeProfiles:vi.fn(),listProviderModels:vi.fn()}));
vi.mock("$lib/server/bff.js",()=>({getBff:()=>bff}));
import {load,actions} from "../routes/tasks/+page.server.js";
const project={id:"project:1",workspaceId:"workspace:1",displayName:"Project",goal:"Goal",state:"active",repositoryConnectionIds:["repository:1","repository:removed"],resourceVersion:"1"};
const workspace={id:"workspace:1",displayName:"Workspace",state:"active",repositoryConnectionIds:["repository:1"],resourceVersion:"1"};
const page={nextPageToken:"",snapshotCursor:"1"};
beforeEach(()=>{vi.clearAllMocks();bff.getProject.mockResolvedValue(project);bff.listProjects.mockResolvedValue({projects:[project],page});bff.getWorkspace.mockResolvedValue(workspace);bff.listDevelopmentTasks.mockResolvedValue({developmentTasks:[],page});bff.listRuntimeProfiles.mockResolvedValue({runtimeProfiles:[],page});bff.listProviderModels.mockResolvedValue({models:[],page});});
function event(){return {locals:{requestId:"task-workspace",requestSignal:new AbortController().signal},url:new URL("http://localhost/tasks"),setHeaders:vi.fn(),depends:vi.fn()} as never;}
describe("task workspace choices",()=>{
  it("offers only repositories still owned by the project's workspace",async()=>{expect(await load(event())).toMatchObject({projects:[{...project,repositoryConnectionIds:["repository:1"]}]});});
  it("excludes projects in archived workspaces",async()=>{bff.getWorkspace.mockResolvedValueOnce({...workspace,state:"archived"});expect(await load(event())).toMatchObject({projects:[]});});
  it("excludes projects with no remaining usable workspace repositories",async()=>{bff.getWorkspace.mockResolvedValueOnce({...workspace,repositoryConnectionIds:["repository:other"]});expect(await load(event())).toMatchObject({projects:[]});});
  it.each(["archived","removed"])("rejects a direct task POST for %s workspace access",async condition=>{
    bff.getWorkspace.mockResolvedValueOnce(condition==="archived"?{...workspace,state:"archived"}:{...workspace,repositoryConnectionIds:[]});
    const form=new FormData();for(const [key,value] of Object.entries({idempotencyKey:"00000000-0000-4000-8000-000000000001",projectId:project.id,repositoryConnectionId:"repository:1",runtimeProfileId:"profile:1",provider:"openai",model:"coding",request:"Ship work",acceptanceCriteria:"Behavior works"}))form.set(key,value);
    expect(await actions.create!({locals:{requestId:"task-workspace",requestSignal:new AbortController().signal},request:new Request("http://localhost/tasks",{method:"POST",body:form})} as never)).toMatchObject({status:409});
    expect(bff.getRuntimeProfile).not.toHaveBeenCalled();expect(bff.createDevelopmentTask).not.toHaveBeenCalled();
  });
});
