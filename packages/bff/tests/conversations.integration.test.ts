import {Server,ServerCredentials,type UntypedServiceImplementation} from "@grpc/grpc-js";
import {afterEach,beforeEach,describe,expect,it} from "vitest";
import {ProjectConversationServiceService,ProjectConversation,ProviderServiceService} from "@ayni-orchestrator/grpc-bindings/server";
import {OrchestratorBff} from "../src/index.js";
const context={requestId:"conversation-integration"};
describe("project conversation native gRPC boundary",()=>{
  let server:Server,bff:OrchestratorBff;
  let requests:Array<{method:string;request:unknown}>;
  const conversation=()=>ProjectConversation.fromPartial({id:"conversation:1",projectId:"project:1",title:"Research",repositoryConnectionIds:["repository:1"],updateCursor:"9",createdAt:new Date("2026-10-13T00:00:00Z"),messages:[{id:"message:1",role:"assistant",content:"Review the plan",createdAt:new Date("2026-10-13T00:00:00Z"),runId:"run:1"}],runs:[{id:"run:1",state:"completed",model:"available-model",connectionId:"connection:1",credentialRevision:"private-revision",modelCalls:3,providerCredentials:{secretName:"private-secret-reference",secretUid:"private-secret-uid"},tokenUsage:{totalTokens:"123"}}],proposals:[{id:"proposal:1",action:"create_task",state:"pending",runId:"run:1",request:{projectId:"project:1",repositoryConnectionId:"repository:1",repositoryRevision:"a".repeat(40),runtimeProfileId:"runtime:1",connectionId:"connection:1",agent:{provider:"openai",agent:"coding-agent",model:"available-model"},request:"Implement the behavior",acceptanceCriteria:["Behavior observable"],codingTurns:24,changeConstraints:{protectedPaths:["policy.toml"],allowTestEdits:true},deadline:{seconds:"3600"}},researchedRuntime:{runtimeProfileRevisionId:"revision:1",runtimeArtifactId:"artifact:1",ociRepository:"ghcr.io/example/runtime",resolvedDigest:`sha256:${"b".repeat(64)}`}}]});
  beforeEach(async()=>{
    requests=[];server=new Server();
    const methods:UntypedServiceImplementation={};
    for(const method of ["createProjectConversation","getProjectConversation","sendProjectMessage","decideProjectProposal"]){methods[method]=(call:any,callback:any)=>{requests.push({method,request:call.request});callback(null,{conversation:conversation()});};}
    methods.listProjectConversations=(call:any,callback:any)=>{requests.push({method:"listProjectConversations",request:call.request});callback(null,{conversations:[conversation()]});};
    methods.watchProjectConversation=(call:any)=>{requests.push({method:"watchProjectConversation",request:call.request});call.write({conversation:conversation()});call.end();};
    server.addService(ProjectConversationServiceService,methods);
    server.addService(ProviderServiceService,{listProviderModels:(call:any,callback:any)=>{requests.push({method:"listProviderModels",request:call.request});callback(null,{connectionId:call.request.connectionId,provider:"openai",models:[{id:"available-model"}],observedAt:new Date("2026-10-13T00:00:00Z"),page:{nextPageToken:"",snapshotCursor:"1"}});}});
    const port=await new Promise<number>((resolve,reject)=>server.bindAsync("127.0.0.1:0",ServerCredentials.createInsecure(),(error,port)=>error?reject(error):resolve(port)));
    bff=new OrchestratorBff({address:`127.0.0.1:${port}`});
  });
  afterEach(async()=>{bff?.close();await new Promise<void>(resolve=>server.tryShutdown(()=>resolve()));});
  it("preserves complete proposal requirements and excludes credential references from browser DTOs",async()=>{
    const result=await bff.getProjectConversation("conversation:1",context);
    expect(result.proposals[0]?.request).toMatchObject({connectionId:"connection:1",repositoryRevision:"a".repeat(40),codingTurns:24,deadlineSeconds:3600,changeConstraints:{protectedPaths:["policy.toml"],allowTestEdits:true}});
    expect(result.proposals[0]?.runtime?.profileRevisionId).toBe("revision:1");
    expect(result.runs[0]?.usage?.totalTokens).toBe("123");
    expect(JSON.stringify(result)).not.toContain("private-");
  });
  it("preserves conversation membership and action idempotency keys",async()=>{
    await bff.createProjectConversation({idempotencyKey:"create-key",projectId:"project:1",title:"Research",repositoryConnectionIds:["repository:1","repository:2"]},context);
    await bff.sendProjectMessage({idempotencyKey:"message-key",conversationId:"conversation:1",content:"Requested revision"},context);
    await bff.decideProjectProposal({idempotencyKey:"approval-key",conversationId:"conversation:1",proposalId:"proposal:1",decision:"approve"},context);
    expect(requests.map(r=>r.request)).toEqual([
      {idempotencyKey:"create-key",projectId:"project:1",title:"Research",repositoryConnectionIds:["repository:1","repository:2"]},
      {idempotencyKey:"message-key",conversationId:"conversation:1",content:"Requested revision"},
      {idempotencyKey:"approval-key",conversationId:"conversation:1",proposalId:"proposal:1",decision:"approve"},
    ]);
  });
  it("discovers models for the selected connection and resumes only cursor invalidations",async()=>{
    const models=await bff.listProviderModels({pageSize:100},context,"connection:1");
    expect(models.connectionId).toBe("connection:1");
    const cursors:string[]=[];await bff.watchProjectConversation("conversation:1","8",context,cursor=>{cursors.push(cursor);});
    expect(cursors).toEqual(["9"]);
    expect(requests.at(-1)?.request).toEqual({conversationId:"conversation:1",afterCursor:"8"});
  });
});
