import { createServer } from "node:http";
import { Server, ServerCredentials, status, type ServerWritableStream, type UntypedServiceImplementation } from "@grpc/grpc-js";
import {
  CertificateServiceService, Certificate,
  ConnectionServiceService, type Connection,
  ProjectServiceService, ProjectState, WorkspaceState,
  ProjectConversationServiceService, ProjectConversation,
  DevelopmentAttemptState,
  DevelopmentTaskServiceService,
  DevelopmentTaskState,
  ModelUsageCollectionStatus,
  ProviderServiceService,
  RepositoryConnectionStatus,
  RepositoryServiceService,
  RuntimeProfileKind,
  RuntimeProfileServiceService,
  RuntimeProfileState,
  RuntimeProfileSupport,
} from "@ayni-orchestrator/grpc-bindings/server";

const taskId = "00000000-0000-4000-8000-000000000039";
const repositoryId = "repository:1";
const profileId = "profile:1";
const revision = "a".repeat(40);
const digest = `sha256:${"b".repeat(64)}`;
const now = new Date("2026-09-28T10:00:00.000Z");
let version = 10;

let repository: any = {
  id: repositoryId,
  provider: { provider: { $case: "github", github: { repositoryId: "123", owner: "ayni-oss", name: "sandbox", installationId: "42", htmlUrl: "https://github.com/ayni-oss/sandbox" } } },
  displayName: "ayni-oss/sandbox",
  defaultBranch: "main",
  onboardingStatus: "awaiting_trust",
  onboardingReason: "Merge the trust proposal before preparing the runtime.",
  onboardingRevision: revision,
  status: RepositoryConnectionStatus.REPOSITORY_CONNECTION_STATUS_CONNECTED,
  createdAt: now,
  updatedAt: now,
  resourceVersion: "1",
};

const secondRepository = {...repository,id:"repository:2",displayName:"ayni-oss/library",provider:{provider:{$case:"github",github:{...repository.provider.provider.github,repositoryId:"456",name:"library",htmlUrl:"https://github.com/ayni-oss/library"}}}};
let workspace:any={plannerIdleTimeoutSeconds:300,id:"workspace:1",displayName:"Shared workspace",state:WorkspaceState.WORKSPACE_STATE_ACTIVE,repositoryConnectionIds:[repositoryId,"repository:2"],resourceVersion:"1",createdAt:now,updatedAt:now};
let project: any = {workspaceId:workspace.id,goal:"Improve the sandbox",id:"project:1", displayName:"Sandbox", state:ProjectState.PROJECT_STATE_ACTIVE, repositoryConnectionIds:[repositoryId], resourceVersion:"1", createdAt:now, updatedAt:now};
const profile: any = {
  id: profileId,
  repositoryConnectionId: repositoryId,
  kind: RuntimeProfileKind.RUNTIME_PROFILE_KIND_AYNI_MANAGED,
  support: RuntimeProfileSupport.RUNTIME_PROFILE_SUPPORT_OPERATIONAL,
  state: RuntimeProfileState.RUNTIME_PROFILE_STATE_READY,
  activeRevisionId: "profile-revision:1",
  activeRuntimeArtifactId: "artifact:1",
  reconciliationMetadata: [], findings: [], preparationPhase: "ready", rebuildAvailable: true,
  createdAt: now,
  updatedAt: now,
  resourceVersion: "2",
};

function snapshot(request = "Correct the bounded retry formula", sourceRevision = revision, model = "gpt-5.6-sol"): any {
  return {
    limits: {codingTurns:24,qualityRepairLimit:3},
    repository: { repositoryConnectionId: repositoryId, repositorySnapshotId: "snapshot:1", revision: sourceRevision },
    runtime: {
      runtimeProfileId: profileId,
      runtimeProfileRevisionId: "profile-revision:1",
      profileKind: RuntimeProfileKind.RUNTIME_PROFILE_KIND_AYNI_MANAGED,
      runtimeArtifactId: "artifact:1",
      ociRepository: "ghcr.io/ayni-oss/sandbox",
      resolvedDigest: digest,
      platform: { operatingSystem: "linux", architecture: "amd64" },
    },
    agent: { provider: "openai", agent: "coding-agent", model },
    request,
    acceptanceCriteria: ["Three retries produce eight attempts", "Existing checks remain green"],
    changeConstraints: { protectedPaths: [".github/workflows"], allowTestEdits: false },
    deadlineAt: new Date("2026-09-28T11:00:00.000Z"),
  };
}

function task(id = taskId, state = DevelopmentTaskState.DEVELOPMENT_TASK_STATE_RUNNING, request?: string): any {
  return {
    id,
    projectId:"project:1",
    snapshot: snapshot(request),
    state,
    activeAttemptId: `attempt:${id}`,
    attemptCount: 1,
    createdAt: now,
    updatedAt: now,
    resourceVersion: String(version++),
  };
}

const tasks = new Map<string, any>();
tasks.set(taskId, task());
tasks.set("00000000-0000-4000-8000-000000000044", task("00000000-0000-4000-8000-000000000044", DevelopmentTaskState.DEVELOPMENT_TASK_STATE_FAILED, "Retained final work awaiting publication"));
const publicationRetries = new Set<string>();
tasks.set("00000000-0000-4000-8000-000000000043", task("00000000-0000-4000-8000-000000000043", DevelopmentTaskState.DEVELOPMENT_TASK_STATE_RUNNING, "Active task for cancellation"));

const page = { nextPageToken: "", snapshotCursor: "10" };
const repositoryService: UntypedServiceImplementation = {
  analyzeRepository(_call: any, callback: any) {
    repository = { ...repository, onboardingStatus: "analyzing", onboardingReason: "Inspecting the committed policy and lock.", resourceVersion: String(version++) };
    callback(null, { repositoryConnection: repository });
  },
  listGithubInstallations(_call, callback) { callback(null, { installations: [{ installationId: "42", owner: "ayni-oss", appName: "Ayni" }] }); },
  listGithubInstallationRepositories(_call, callback) { callback(null, { repositories: [{ repositoryId: "123", owner: "ayni-oss", name: "sandbox", defaultBranch: "main" }] }); },
  setRepositoryRegistry(call,callback){repository={...repository,registryConnectionId:call.request.registryConnectionId,imageNamespace:call.request.imageNamespace,resourceVersion:String(version++)};callback(null,{repositoryConnection:repository});},
  connectGithubRepository(_call,callback){callback(null,{repositoryConnection:repository});},

  listRepositorySummaries(_call,callback) { callback(null,{summaries:[repository,secondRepository].map(item=>({repositoryConnection:item,runtime:runtimeSummary(item.id)})),page}); },
  listRepositoryConnections(_call, callback) { callback(null, { repositoryConnections: [repository,secondRepository], page }); },
  getRepositoryConnection(call, callback) { callback(null, { repositoryConnection: call.request.repositoryConnectionId === secondRepository.id ? secondRepository : repository }); },
  createRepositoryConnection(call, callback) {
    repository = { ...repository, provider: call.request.provider, displayName: call.request.displayName, defaultBranch: call.request.defaultBranch, updatedAt: new Date(), resourceVersion: String(version++) };
    callback(null, { repositoryConnection: repository });
  },
  listRepositorySnapshots(_call, callback) { callback(null, { repositorySnapshots: [{ id: "snapshot:1", repositoryConnectionId: repositoryId, revision, defaultBranch: "main", observedAt: now, resourceVersion: "3" }], page }); },
  getRepositorySnapshot(_call, callback) { callback(null, { repositorySnapshot: { id: "snapshot:1", repositoryConnectionId: repositoryId, revision, defaultBranch: "main", observedAt: now, resourceVersion: "3" } }); },
  watchRepositories(call) { call.write({ cursor: "10", update: { $case: "repositoryConnection", repositoryConnection: repository } }); call.end(); },
};

function profileView() {
  return {...profile,effectiveCertificate:certificates.find(c=>c.id===profile.certificateId) ?? certificates.find(c=>c.isDefault),desiredImageRepository:"ghcr.io/ayni-oss/sandbox"};
}

let profileEnrolled = true;
function runtimeSummary(repository:string) {
  const operational=profileEnrolled && repository===repositoryId && profile.support===RuntimeProfileSupport.RUNTIME_PROFILE_SUPPORT_OPERATIONAL;
  const ready=operational && profile.state===RuntimeProfileState.RUNTIME_PROFILE_STATE_READY;
  return {complete:true,ready,operationalProfiles:operational ? 1 : 0,readyProfiles:ready ? 1 : 0};
}

const runtimeProfileService: UntypedServiceImplementation = {
  inspectAyniRepository(_call,callback){callback(null,{repositorySnapshotId:"snapshot:1",repositoryRevision:revision,lockFingerprint:"c".repeat(64),ayniVersion:"0.14.5",ociRepository:"ghcr.io/ayni-oss/sandbox",policyPresent:true,lockCompatible:true,trustPolicyPinned:true,reason:"signing_configuration_requires_operator"});},
  createRuntimeProfile(_call,callback){profileEnrolled=true;profile.state=RuntimeProfileState.RUNTIME_PROFILE_STATE_SETUP_REQUIRED;callback(null,{runtimeProfile:profileView()});},
  setRuntimeCertificate(call,callback){
    if(call.request.expectedResourceVersion!==profile.resourceVersion) return callback({code:status.ABORTED,message:"Refresh the runtime and try again."});
    profile.certificateId=call.request.certificateId;profile.resourceVersion=String(Number(profile.resourceVersion)+1);
    callback(null,{runtimeProfile:profileView()});
  },
  setRuntimeRegistry(call,callback){
    if(call.request.expectedResourceVersion!==profile.resourceVersion) return callback({code:status.ABORTED,message:"Refresh the runtime and try again."});
    const connection=connections.find(item=>item.id===call.request.registryConnectionId);
    if(!connection || connection.kind!=="ghcr" || !connection.enabled || connection.status!=="ready") return callback({code:status.FAILED_PRECONDITION,message:"Choose a ready GHCR connection."});
    profile.registry={registryConnectionId:connection.id};
    profile.resourceVersion=String(Number(profile.resourceVersion)+1);
    callback(null,{runtimeProfile:profileView()});
  },
  reconcileRuntimeProfile(_call,callback){profile.findings=[];profile.preparationPhase="ready";profile.rebuildAvailable=true;profile.failure=undefined;profile.resourceVersion=String(Number(profile.resourceVersion)+1);profile.state=RuntimeProfileState.RUNTIME_PROFILE_STATE_READY;callback(null,{runtimeProfile:profileView()});},

  getRuntimeProfileRevision(_call,callback) { callback(null,{revision:{id:"profile-revision:1",runtimeProfileId:profileId,repositoryRevision:revision,repositorySnapshotId:"snapshot:1",revisionNumber:"1",resourceVersion:"1",configuration:{$case:"ayniManaged",ayniManaged:{lockFingerprint:"c".repeat(64),ayniVersion:"0.14.5",ociRepository:"ghcr.io/ayni-oss/sandbox"}}}}); },

  listRuntimeProfiles(call, callback) {
    const profiles = profileEnrolled && (call.request.repositoryConnectionId === undefined || call.request.repositoryConnectionId === repositoryId) ? [profileView()] : [];
    callback(null, { runtimeProfiles: profiles, page });
  },
  getRuntimeProfile(_call, callback) { callback(null, { runtimeProfile: profileView() }); },
  listRuntimeArtifacts(_call, callback) { callback(null, { runtimeArtifacts: [], page }); },
};

const providerService: UntypedServiceImplementation = {
  listProviderModels(_call, callback) { callback(null, {connectionId:_call.request.connectionId, provider: "openai", models: [{ id: "gpt-5.6-sol", name: "Sol", ownedBy: "openai", contextLength: "400000", maxOutputTokens: "128000" },{id:"gpt-5.6-luna",name:"Luna",ownedBy:"openai"}], observedAt: now, page }); },
};

const taskService: UntypedServiceImplementation = {
  listDevelopmentAttempts(call, callback) {
    const t = tasks.get(call.request.developmentTaskId);
    const publicationFailed = t?.id.endsWith("0044");
    callback(null, { attempts: t ? [{publicationOnly:publicationRetries.has(t.id), id: `attempt:${t.id}`, developmentTaskId:t.id, attemptNumber:1, reportingState:"accepted", cleanupState:"complete", snapshot:t.snapshot, state:DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_SUCCEEDED, agentActivity:[], progress:{phase:8,codingTurnsConsumed:5,qualityRound:2,publication:3}, result:{execution:1,commit:"d".repeat(40),changedPaths:["src/main.rs"],quality:{outcome:2,completedTargets:1,skippedTargets:0,warningOffenders:0,failingOffenders:1,rows:[{kind:"test",passed:false,status:"fail",summary:"total=4 passed=3 failed=1",target:"rust (workspace)"},{kind:"size",passed:true,status:"warn",summary:"max_lines=900 fail_count=0",target:"rust (workspace)"}],findings:["Repair the failing test"],evidence:{artifactPath:".ayni/last/signals.json",signalsDigest:"e".repeat(64),sourceFingerprint:"f".repeat(64),commit:"d".repeat(40)}},publication:publicationFailed ? 4 : 3,codingTurnsConsumed:5,qualityRound:2,pullRequest:publicationFailed ? undefined : {repository:"ayni-oss/sandbox",number:"9",url:"https://github.com/ayni-oss/sandbox/pull/9",branch:"task/example",baseBranch:"main",headCommit:"d".repeat(40),draft:true}},createdAt:now,updatedAt:now,resourceVersion:"1"}] : [], page});
  },
  retryTaskPublication(call, callback) { const t=tasks.get(call.request.developmentTaskId); publicationRetries.add(t.id); callback(null,{attempt:{id:"publication:retry",developmentTaskId:t.id,attemptNumber:2,snapshot:t.snapshot,state:DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_QUEUED,agentActivity:[],publicationOnly:true,createdAt:now,updatedAt:now,resourceVersion:"2"}}); },

  listDevelopmentTasks(call, callback) { callback(null, { developmentTasks: [...tasks.values()].filter(task=>!call.request.projectId || task.projectId===call.request.projectId), page }); },
  getDevelopmentTask(call, callback) { callback(null, { developmentTask: tasks.get(call.request.developmentTaskId) ?? tasks.get(taskId) }); },
  createDevelopmentTask(call, callback) {
    const id = "00000000-0000-4000-8000-000000000040";
    const request = call.request.request;
    const created = task(id, DevelopmentTaskState.DEVELOPMENT_TASK_STATE_QUEUED, request.request);
    created.snapshot = {
      ...snapshot(request.request, request.repositoryRevision, request.agent.model),
      limits:{codingTurns:request.codingTurns ?? 24,qualityRepairLimit:3},
      publicationBaseBranch:"main",
      agent: request.agent,
      acceptanceCriteria: request.acceptanceCriteria,
      changeConstraints: request.changeConstraints,
      ...(request.deadline === undefined ? {} : { deadlineAt: new Date(now.getTime() + Number(request.deadline.seconds) * 1000) }),
    };
    tasks.set(id, created);
    callback(null, { developmentTask: created });
  },
  cancelDevelopmentTask(call, callback) {
    const value = tasks.get(call.request.developmentTaskId);
    value.state = DevelopmentTaskState.DEVELOPMENT_TASK_STATE_CANCELLED;
    value.updatedAt = new Date();
    value.resourceVersion = String(version++);
    callback(null, { developmentTask: value });
  },
  retryDevelopmentTask(call, callback) {
    const value = tasks.get(call.request.developmentTaskId);
    value.state = DevelopmentTaskState.DEVELOPMENT_TASK_STATE_QUEUED;
    value.attemptCount += 1;
    value.resourceVersion = String(version++);
    callback(null, { attempt: { id: `attempt:${value.id}:${value.attemptCount}`, developmentTaskId: value.id, attemptNumber: value.attemptCount, snapshot: value.snapshot, state: DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_QUEUED, modelUsage: { collectionStatus: ModelUsageCollectionStatus.MODEL_USAGE_COLLECTION_STATUS_UNAVAILABLE, interactions: [], byStage: [], total: { interactionCount: 0, unavailableInteractionCount: 0 } }, createdAt: new Date(), updatedAt: new Date(), resourceVersion: value.resourceVersion } });
  },
  watchDevelopmentTask(call) {
    const value = tasks.get(call.request.developmentTaskId);
    if (value !== undefined) call.write({ cursor: value.resourceVersion, update: { $case: "developmentTask", developmentTask: value } });
    call.end();
  },
};

const grpc = new Server();
let connections:Connection[]=[];
let rejectConnectionMutation = false;
function rejectMutation(callback: (error: { code: number; message: string }) => void): boolean {
  if (!rejectConnectionMutation) return false;
  rejectConnectionMutation = false;
  callback({ code: 9, message: "fixture credential validation failed" });
  return true;
}
const connectionService:UntypedServiceImplementation={
  listConnections(_call,callback){callback(null,{connections});},
  createConnection(call,callback){if(rejectMutation(callback))return;const r=call.request;const connection:Connection={id:`connection:${connections.length+1}`,kind:r.kind,managementSource:"ui",displayName:r.displayName,appId:r.appId,username:r.username,imageNamespace:r.imageNamespace,secretName:"safe-reference",secretUid:"safe-uid",desiredRevision:"1",observedRevision:"1",status:"ready",failureReason:"",enabled:true,resourceVersion:"1",pullSecretName:"safe-pull",testAuthentication:"passed",testPush:"passed",testPull:"passed",testCleanup:"passed",testArtifactDigest:"",candidateSecretName:"",candidateSecretUid:"",candidatePullSecretName:"",candidatePullSecretUid:"",pullSecretUid:"",retiredSecretRevisions:[],testCleanupFailureReason:"",testArtifactRepository:""};connections.push(connection);callback(null,{connection});},
  replaceConnectionCredentials(call,callback){if(rejectMutation(callback))return;const connection=connections.find(c=>c.id===call.request.id);if(connection){if(call.request.expectedResourceVersion!==connection.resourceVersion){callback({code:10,message:"fixture stale revision"});return;}connection.resourceVersion=String(Number(connection.resourceVersion)+1);callback(null,{connection});}else callback({code:5,message:"Connection not found"});},
  testConnection(call,callback){callback(null,{connection:connections.find(c=>c.id===call.request.id)});},
  setConnectionEnabled(call,callback){const connection=connections.find(c=>c.id===call.request.id);if(connection){connection.enabled=call.request.enabled;connection.status=connection.enabled?"ready":"disabled";}callback(null,{connection});},
  deleteConnection(call,callback){connections=connections.filter(c=>c.id!==call.request.id);callback(null,{});},
  completeGithubManifest(_call,callback){callback({code:3,message:"Invalid manifest code"});},
};
let certificates:Certificate[]=[];
function certificate(data:Partial<Certificate>={}):Certificate {return {id:`certificate:${certificates.length+1}`,managementSource:"ui",displayName:"Signing key",keyId:"team-key",publicKey:"b".repeat(64),secretName:"fixture-signing",secretUid:"fixture-uid",revision:"1",enabled:true,isDefault:certificates.length===0,resourceVersion:"1",status:"ready",failureReason:"",sourceSecretName:"",sourceSecretUid:"",sourceRevision:"",retiredSecretRevisions:[],usageCount:"0",...data};}
function certificateMutation(operation:string):UntypedServiceImplementation[string] {return(call:any,callback:any)=>{
  if(rejectMutation(callback))return;
  const c=certificates.find(c=>c.id===call.request.id);
  if(!c)return callback({code:status.NOT_FOUND,message:"Certificate not found"});
  if(c.resourceVersion!==call.request.expectedResourceVersion)return callback({code:status.ABORTED,message:"Refresh certificate"});
  if(operation==="replace"){c.keyId=call.request.keyId;c.revision=String(Number(c.revision)+1);}
  if(operation==="rename")c.displayName=call.request.displayName;
  if(operation==="toggle")c.enabled=call.request.enabled;
  if(operation==="default")certificates.forEach(item=>item.isDefault=item.id===c.id);
  if(operation==="delete"){certificates=certificates.filter(item=>item.id!==c.id);return callback(null,{});}
  c.resourceVersion=String(Number(c.resourceVersion)+1);callback(null,{certificate:c});
};}
function resourceWatch(call: ServerWritableStream<any, any>, field: string, values: () => any[]) {
  let previous = JSON.stringify(values());
  let cursor = 1;
  for (const value of values()) call.write({ cursor: String(cursor), [field]: value, deleted: false, snapshotComplete: false });
  call.write({ cursor: String(cursor), snapshotComplete: true, deleted: false });
  const timer = setInterval(() => {
    const current = JSON.stringify(values());
    if (current === previous) return;
    previous = current;
    cursor++;
    // Browser only consumes invalidations. The durable stream contract is tested
    // against the real SQLite adapter, including tombstones and replay.
    call.write({ cursor: String(cursor), snapshotComplete: true, deleted: false });
  }, 100);
  call.on("cancelled", () => clearInterval(timer));
  call.on("close", () => clearInterval(timer));
}
function projectionWatch(call: ServerWritableStream<any, any>, field: string, values: () => any[]) {
  let previous = "";
  let cursor = 1;
  function send() {
    const current = JSON.stringify(values());
    if (current === previous) return;
    previous = current;
    for (const value of values()) call.write({ cursor: String(cursor++), update: { $case: field, [field]: value } });
  }
  send();
  const timer = setInterval(send, 100);
  call.on("cancelled", () => clearInterval(timer));
  call.on("close", () => clearInterval(timer));
}
repositoryService.watchRepositories = call => projectionWatch(call, "repositoryConnection", () => [repository]);
runtimeProfileService.watchRuntimeProfiles = call => projectionWatch(call, "runtimeProfile", () => profileEnrolled ? [profileView()] : []);
connectionService.watchConnections = call => resourceWatch(call, "connection", () => connections);
grpc.addService(CertificateServiceService,{
  watchCertificates(call:any) { resourceWatch(call, "certificate", () => certificates); },
  listCertificates(_call:any,callback:any){callback(null,{certificates});},
  getCertificate(call:any,callback:any){callback(null,{certificate:certificates.find(c=>c.id===call.request.id)});},
  createCertificate(call:any,callback:any){if(rejectMutation(callback))return;const c=certificate({displayName:call.request.displayName,keyId:call.request.keyId});certificates.push(c);callback(null,{certificate:c});},
  replaceCertificate:certificateMutation("replace"),renameCertificate:certificateMutation("rename"),setCertificateEnabled:certificateMutation("toggle"),setDefaultCertificate:certificateMutation("default"),deleteCertificate:certificateMutation("delete"),
});
grpc.addService(ConnectionServiceService,connectionService);
grpc.addService(ProjectServiceService, {
  createWorkspace(call:any,callback:any){workspace={...workspace,displayName:call.request.displayName,repositoryConnectionIds:call.request.repositoryConnectionIds,state:WorkspaceState.WORKSPACE_STATE_ACTIVE,resourceVersion:"1"};callback(null,{workspace});},
  listWorkspaces(_call:any,callback:any){callback(null,{workspaces:[workspace],page});},
  getWorkspace(_call:any,callback:any){callback(null,{workspace});},
  updateWorkspace(call:any,callback:any){workspace={...workspace,...(call.request.plannerIdleTimeoutSeconds===undefined?{}:{plannerIdleTimeoutSeconds:call.request.plannerIdleTimeoutSeconds}),...(call.request.agentDefaults?{agentDefaults:call.request.agentDefaults}:{}),displayName:call.request.displayName,repositoryConnectionIds:call.request.repositoryConnectionIds.length?call.request.repositoryConnectionIds:workspace.repositoryConnectionIds,resourceVersion:String(Number(workspace.resourceVersion)+1)};callback(null,{workspace});},
  archiveWorkspace(_call:any,callback:any){workspace={...workspace,state:WorkspaceState.WORKSPACE_STATE_ARCHIVED,resourceVersion:String(Number(workspace.resourceVersion)+1)};callback(null,{workspace});},
  watchProjects(call:any) { resourceWatch(call, "project", () => [project]); },
  listProjectSummaries(_call:any,callback:any){
    const members=project.repositoryConnectionIds.map((id:string)=>runtimeSummary(id));
    const latest=[...tasks.values()].filter(task=>task.projectId===project.id).sort((a,b)=>b.createdAt.getTime()-a.createdAt.getTime() || b.id.localeCompare(a.id))[0];
    callback(null,{summaries:[{project,repositoryNames:project.repositoryConnectionIds.map((id:string)=>id===repositoryId ? repository.displayName : secondRepository.displayName),runtime:{complete:true,ready:members.every((member:any)=>member.ready),operationalProfiles:members.reduce((n:number,member:any)=>n+member.operationalProfiles,0),readyProfiles:members.reduce((n:number,member:any)=>n+member.readyProfiles,0)},tasksComplete:true,latestTask:latest ? {id:latest.id,state:latest.state,request:latest.snapshot.request,createdAt:latest.createdAt} : undefined}],page});
  },
  listProjects(_call:any,callback:any){callback(null,{projects:[project],page});},
  getProject(_call:any,callback:any){callback(null,{project});},
  createProject(call:any,callback:any){project={...project,workspaceId:call.request.workspaceId,goal:call.request.goal,displayName:call.request.displayName,repositoryConnectionIds:call.request.repositoryConnectionIds.length ? call.request.repositoryConnectionIds : [call.request.repositoryConnectionId],state:ProjectState.PROJECT_STATE_ACTIVE,resourceVersion:"1"};callback(null,{project});},
  updateProject(call:any,callback:any){project={...project,...(call.request.goal!==undefined?{goal:call.request.goal}:{}),...(call.request.agentDefaults ? {agentDefaults:call.request.agentDefaults} : {}),displayName:call.request.displayName,repositoryConnectionIds:call.request.repositoryConnectionIds.length ? call.request.repositoryConnectionIds : project.repositoryConnectionIds,resourceVersion:String(Number(project.resourceVersion)+1)};callback(null,{project});},
  archiveProject(_call:any,callback:any){project={...project,state:ProjectState.PROJECT_STATE_ARCHIVED,resourceVersion:String(Number(project.resourceVersion)+1)};callback(null,{project});},
});
// Conversation fixtures exercise real generated gRPC and browser/BFF boundaries.
// LLM and Kubernetes behavior is validated by native and cluster tests.
let conversations:ProjectConversation[]=[];
let stalePlan=false;
const proposalTasks=new Map<string,string>();
function conversationProjection(c:ProjectConversation){return {...c,effectivePlannerIdleTimeoutSeconds:c.plannerIdleTimeoutSeconds??workspace.plannerIdleTimeoutSeconds??300};}
function changedConversation(c:ProjectConversation){c.updateCursor=String(Number(c.updateCursor)+1);}
grpc.addService(ProjectConversationServiceService,{
  updateProjectConversationSettings(call:any,callback:any){
    const c=conversations.find(c=>c.id===call.request.conversationId&&c.projectId===call.request.projectId);
    if(!c)return callback({code:status.NOT_FOUND,message:"Conversation not found"});
    if(c.updateCursor!==call.request.expectedUpdateCursor)return callback({code:status.ABORTED,message:"Conversation settings changed. Reload and try again."});
    if(call.request.inheritWorkspaceTimeout){delete c.plannerIdleTimeoutSeconds;}else{if(![300,86400].includes(call.request.plannerIdleTimeoutSeconds))return callback({code:status.INVALID_ARGUMENT,message:"Choose a supported timeout"});c.plannerIdleTimeoutSeconds=call.request.plannerIdleTimeoutSeconds;}
    changedConversation(c);callback(null,{conversation:conversationProjection(c)});
  },
  cancelProjectAgentRun(call:any,callback:any){
    const c=conversations.find(c=>c.id===call.request.conversationId&&c.projectId===call.request.projectId),run=c?.runs.find(r=>r.id===call.request.runId);
    if(!c||!run)return callback({code:status.NOT_FOUND,message:"Planner turn not found"});
    run.state="interrupted";run.failure="Planner turn cancelled by operator";
    if(c.plannerSession)c.plannerSession={...c.plannerSession,state:"expired",activeRunId:""};
    changedConversation(c);callback(null,{conversation:conversationProjection(c)});
  },
  getProjectConversationContext(call:any,callback:any){
    const c=conversations.find(c=>c.id===call.request.conversationId&&c.projectId===call.request.projectId);
    if(!c)return callback({code:status.NOT_FOUND,message:"Conversation not found"});
    callback(null,{sourceWatermark:String(c.messages.length),archiveRecordCount:String(c.messages.length),capacityRecordLimit:"100000",recentMessageLimit:32,...(c.messages.length?{summary:{version:"2",sourceWatermark:String(c.messages.length),content:"Keep the original compatibility constraint in the plan.",sourceRecordIds:[`record:${c.messages[0]!.id}`]}}:{})});
  },
  listProjectConversationHistory(call:any,callback:any){
    const c=conversations.find(c=>c.id===call.request.conversationId&&c.projectId===call.request.projectId);
    if(!c)return callback({code:status.NOT_FOUND,message:"Conversation not found"});
    const snapshot=Number(call.request.snapshotSequence)||c.messages.length,before=Number(call.request.beforeSequence)||snapshot+1;
    const matches=c.messages.map((m,i)=>({sequence:String(i+1),id:`record:${m.id}`,kind:"message",sourceId:m.id,repositoryId:"",role:m.role,content:m.content})).filter(r=>Number(r.sequence)<=snapshot&&Number(r.sequence)<before&&r.content.toLowerCase().includes(call.request.query.toLowerCase())).reverse();
    const records=matches.slice(0,call.request.limit||50);
    callback(null,{records,snapshotSequence:String(snapshot),nextBeforeSequence:records.at(-1)?.sequence??"0",hasMore:matches.length>records.length,archiveRecordCount:String(c.messages.length)});
  },
  createProjectConversation(call:any,callback:any){const c=ProjectConversation.fromPartial({id:`conversation:${conversations.length+1}`,projectId:call.request.projectId,title:call.request.title,repositoryConnectionIds:call.request.repositoryConnectionIds,updateCursor:"1",createdAt:now});conversations.push(c);callback(null,{conversation:c});},
  listProjectConversations(_call:any,callback:any){callback(null,{conversations:conversations.map(c=>ProjectConversation.fromPartial({id:c.id,projectId:c.projectId,title:c.title,repositoryConnectionIds:c.repositoryConnectionIds,updateCursor:c.updateCursor,createdAt:c.createdAt}))});},
  getProjectConversation(call:any,callback:any){const c=conversations.find(c=>c.id===call.request.conversationId);if(!c)return callback({code:status.NOT_FOUND,message:"Conversation not found"});callback(null,{conversation:{...conversationProjection(c),messages:c.messages.slice(-32)}});},
  sendProjectMessage(call:any,callback:any){
    const c=conversations.find(c=>c.id===call.request.conversationId)!;
    c.messages.push({id:`message:${c.messages.length+1}`,role:"user",content:call.request.content,createdAt:now,runId:""});
    const runId=`run:${c.runs.length+1}`;
    const generation=c.plannerSession?.state==="idle"?c.plannerSession.generation:String(Number(c.plannerSession?.generation??0)+1);
    c.plannerSession={sessionId:`session:${c.id}`,generation,state:"idle",activeRunId:"",lastActivityAt:now,idleExpiresAt:new Date(now.getTime()+(c.plannerIdleTimeoutSeconds??workspace.plannerIdleTimeoutSeconds??300)*1000)};
    c.runs.push({sessionId:c.plannerSession.sessionId,sessionGeneration:generation,sessionTurnSequence:String(c.runs.length+1),sessionManaged:true,id:runId,state:"completed",model:project.agentDefaults.projectModel,connectionId:project.agentDefaults.projectConnectionId,credentialRevision:"private-credential-revision",modelCalls:3,failure:"",createdAt:now});
    c.messages.push({id:`message:${c.messages.length+1}`,role:"assistant",content:"I read the repository instructions and implementation. Review the complete task plan below.",createdAt:now,runId});
    c.proposals.push({id:`proposal:${c.proposals.length+1}`,runId,state:"pending",action:"create_task",targetTaskId:"",taskId:"",attemptId:"",failure:"",request:{projectId:project.id,repositoryConnectionId:c.repositoryConnectionIds[0]!,repositoryRevision:revision,runtimeProfileId:profileId,connectionId:project.agentDefaults.codingConnectionId,agent:{provider:"openai",agent:"coding-agent",model:call.request.content.includes("model gpt-5.6-luna")?"gpt-5.6-luna":project.agentDefaults.codingModel},request:call.request.content,acceptanceCriteria:["Requested behavior is implemented","Required checks pass"],codingTurns:24,changeConstraints:{protectedPaths:["policy.toml"],allowTestEdits:true},deadline:{seconds:"3600",nanos:0}}});
    c.plan??={id:`plan:${c.id}`,versions:[]};
    const prior=c.plan.versions.at(-1);if(prior?.state==="pending")prior.state="superseded";
    c.plan.versions.push({version:String(c.plan.versions.length+1),runId,state:"pending",findings:"Repository instructions and implementation reviewed.",scope:call.request.content,acceptanceCriteria:["Requested behavior is implemented","Required checks pass"],proposalIds:[c.proposals.at(-1)!.id],dependencyEnvironmentImpact:"",createdAt:now,failure:""});
    changedConversation(c);callback(null,{conversation:c});
  },
  decideProjectPlan(call:any,callback:any){
    const c=conversations.find(c=>c.id===call.request.conversationId)!;
    const v=c.plan?.versions.find(v=>v.version===call.request.version);
    if(c.plan?.id!==call.request.planId||!v||v!==c.plan.versions.at(-1)||v.state!=="pending")return callback({code:status.ABORTED,message:"Reload the latest plan"});
    if(stalePlan)return callback({code:status.FAILED_PRECONDITION,message:"Source or runtime changed"});
    v.state=call.request.decision==="approve"?"approved":call.request.decision==="reject"?"rejected":"revise";
    for(const p of c.proposals.filter(p=>v.proposalIds.includes(p.id))){
      p.state=v.state;
      if(v.state==="approved"){
        const id=proposalTasks.get(p.id)??`00000000-0000-4000-8000-${String(50+proposalTasks.size).padStart(12,"0")}`;proposalTasks.set(p.id,id);
        tasks.set(id,task(id,DevelopmentTaskState.DEVELOPMENT_TASK_STATE_SUCCEEDED,p.request!.request));p.taskId=id;p.attemptId=`attempt:${id}`;
      }
    }
    if(v.state==="approved")c.messages.push({id:`message:${c.messages.length+1}`,role:"assistant",content:"Execution succeeded. Quality checks failed. A draft PR was published; review its findings before merging.",createdAt:now,runId:v.runId});
    changedConversation(c);callback(null,{conversation:c});
  },
  decideProjectProposal(call:any,callback:any){
    const c=conversations.find(c=>c.id===call.request.conversationId)!,p=c.proposals.find(p=>p.id===call.request.proposalId)!;
    if(p.state!=="pending")return callback(null,{conversation:c});
    if(call.request.decision==="approve"){
      p.state="approved";const id=proposalTasks.get(p.id)??`00000000-0000-4000-8000-${String(50+proposalTasks.size).padStart(12,"0")}`;proposalTasks.set(p.id,id);
      const created=task(id,DevelopmentTaskState.DEVELOPMENT_TASK_STATE_SUCCEEDED,p.request!.request);tasks.set(id,created);p.taskId=id;p.attemptId=`attempt:${id}`;
      c.messages.push({id:`message:${c.messages.length+1}`,role:"assistant",content:"Execution succeeded. Quality checks failed. A draft PR was published; review its findings before merging.",createdAt:now,runId:p.runId});
    }else p.state=call.request.decision==="reject"?"rejected":"revise";
    changedConversation(c);callback(null,{conversation:c});
  },
  watchProjectConversation(call:any){
    let cursor=call.request.afterCursor;
    const timer=setInterval(()=>{const c=conversations.find(c=>c.id===call.request.conversationId);if(c&&BigInt(c.updateCursor)>BigInt(cursor)){cursor=c.updateCursor;call.write({conversation:c});}},100);
    call.on("cancelled",()=>clearInterval(timer));call.on("close",()=>clearInterval(timer));
  },
});
grpc.addService(RepositoryServiceService, repositoryService);
grpc.addService(RuntimeProfileServiceService, runtimeProfileService);
grpc.addService(ProviderServiceService, providerService);
grpc.addService(DevelopmentTaskServiceService, taskService);
grpc.bindAsync("127.0.0.1:50061", ServerCredentials.createInsecure(), (error) => { if (error) throw error; });
function certificateFixture(pathname: string): void {
 if(pathname==="/reset-certificates") {certificates=[];profile.certificateId="";profile.kind=RuntimeProfileKind.RUNTIME_PROFILE_KIND_AYNI_MANAGED;rejectConnectionMutation=false;}
 if(pathname==="/admin-certificate") certificates=[certificate({id:"admin:certificate",managementSource:"admin",displayName:"Admin signing",sourceSecretName:"admin-signing"})];
 if(pathname==="/custom-profile") profile.kind=RuntimeProfileKind.RUNTIME_PROFILE_KIND_CUSTOM_RUNTIME;
}
function connectionFixture(url: URL): void {
 if(url.pathname==="/reset-connections") {connections=[];rejectConnectionMutation=false;}
 if(url.pathname==="/reject-connection-mutation") rejectConnectionMutation=true;
 if(url.pathname==="/connection-state" && connections[0]) {
   connections[0].resourceVersion=String(Number(connections[0].resourceVersion)+1);
   connections[0].status=url.searchParams.get("status") ?? "ready";
   connections[0].failureReason=url.searchParams.get("failure") ?? "";
   connections[0].testCleanup=url.searchParams.get("cleanup") ?? "passed";
   connections[0].testCleanupFailureReason=connections[0].testCleanup==="failed" ? "registry_cleanup_permission_required" : "";
 }
 if(url.pathname==="/admin-connection") connections=[{id:"admin:github",kind:url.searchParams.get("kind") ?? "github_app",managementSource:"admin",displayName:"Admin App",appId:"123",username:"registry-auth-account",imageNamespace:url.searchParams.get("kind")==="ghcr" ? url.searchParams.get("namespace") ?? "admin-team" : "",secretName:"admin-ref",secretUid:"",desiredRevision:"1",observedRevision:"1",status:"ready",failureReason:"",enabled:true,resourceVersion:"1",pullSecretName:"",testAuthentication:"",testPush:"",testPull:"",testCleanup:"",testArtifactDigest:"",candidateSecretName:"",candidateSecretUid:"",candidatePullSecretName:"",candidatePullSecretUid:"",pullSecretUid:"",retiredSecretRevisions:[],testCleanupFailureReason:"",testArtifactRepository:""}];
}
function profileFixture(url: URL): void {
 if(url.pathname==="/certificate-trust") {
   const state=url.searchParams.get("state");
   const href=url.searchParams.get("href");
   profile.reconciliationMetadata=[
     ...(state ? [{key:"certificate-trust-state",value:state}] : []),
     ...(href ? [{key:"certificate-trust-pr",value:href}] : []),
   ];
 }
 if(url.pathname==="/preparation") {
   profile.preparationPhase=url.searchParams.get("phase") ?? "ready";
   const retry=url.searchParams.get("retry");
   profile.reconciliationMetadata=retry ? [
     {key:"retry-eligibility",value:retry}, {key:"error-class",value:"execution"},
     {key:"last-error",value:"runtime_verification_failed"}, {key:"failed-executions",value:"3"},
   ] : [];
   const code=url.searchParams.get("code");
   profile.findings=code ? [{code,message:"The image certificate is invalid.",remediation:"Review trust and rebuild."}] : [];
   profile.failure=code ? {reason:1,observedAt:now,message:code,remediation:"Review trust and rebuild."} : undefined;
   profile.rebuildAvailable=url.searchParams.get("rebuild")!=="false";
 }
 if(url.pathname==="/reset-onboarding") { profileEnrolled=false; }
 if(url.pathname==="/profile-version") profile.resourceVersion=String(Number(profile.resourceVersion)+1);
 if(url.pathname==="/profile-state") { const state=Number(url.searchParams.get("value")); if(Number.isInteger(state) && state>=1 && state<=7) profile.state=state; }
}
const health = createServer((request, response) => {
 const url=new URL(request.url ?? "/","http://127.0.0.1");
 if(request.method==="POST") {
   if (url.pathname === "/reset-project") tasks.set("00000000-0000-4000-8000-000000000043", task("00000000-0000-4000-8000-000000000043", DevelopmentTaskState.DEVELOPMENT_TASK_STATE_RUNNING, "Active task for cancellation"));
   if (url.pathname === "/reset-project") {conversations=[];proposalTasks.clear();stalePlan=false;}
   if(url.pathname==="/planner-queued")for(const c of conversations){c.plannerSession=undefined;c.runs.push(ProjectConversation.fromPartial({runs:[{id:"run:queued",state:"queued",model:"gpt-5.6-sol",connectionId:"connection:1",createdAt:now}]}).runs[0]!);changedConversation(c);}
   if(url.pathname==="/archive-workspace")workspace={...workspace,state:WorkspaceState.WORKSPACE_STATE_ARCHIVED};
   if(url.pathname==="/expire-planner")for(const c of conversations){if(c.plannerSession)c.plannerSession={...c.plannerSession,state:"expired",activeRunId:""};changedConversation(c);}
   if(url.pathname==="/planner-active")for(const c of conversations){const run=c.runs.at(-1);if(run&&c.plannerSession){run.state="running";c.plannerSession={...c.plannerSession,state:"active",activeRunId:run.id};changedConversation(c);}}
   if(url.pathname==="/long-history")for(const c of conversations){c.messages=Array.from({length:75},(_,i)=>({id:`message:${i+1}`,role:"user",content:i===0?"Original compatibility constraint: preserve legacy API behavior.":`Later discussion ${i+1}`,createdAt:now,runId:""}));changedConversation(c);}
   if(url.pathname==="/stale-plan")stalePlan=true;
   if(url.pathname==="/new-plan-version")for(const c of conversations){
     const previous=c.plan?.versions.at(-1);if(!previous)continue;
     previous.state="superseded";
     const runId=`run:${c.runs.length+1}`;c.runs.push({...c.runs.at(-1)!,id:runId});
     const proposalIds=previous.proposalIds.map(id=>{const p=c.proposals.find(p=>p.id===id)!;const next={...p,id:`proposal:${c.proposals.length+1}`,runId,state:"pending",taskId:"",attemptId:""};c.proposals.push(next);return next.id;});
     c.plan!.versions.push({...previous,version:String(Number(previous.version)+1),runId,state:"pending",scope:"Newer scope requires a fresh review",proposalIds});
     changedConversation(c);
   }
   if(url.pathname==="/restart-conversations")conversations=conversations.map(c=>ProjectConversation.decode(ProjectConversation.encode(c).finish()));
   if (url.pathname === "/reset-project") workspace={plannerIdleTimeoutSeconds:300,id:"workspace:1",displayName:"Shared workspace",state:WorkspaceState.WORKSPACE_STATE_ACTIVE,repositoryConnectionIds:[repositoryId,"repository:2"],resourceVersion:"1",createdAt:now,updatedAt:now};
   if (url.pathname === "/reset-project") project = {workspaceId:workspace.id,goal:"Improve the sandbox",id:"project:1", displayName:"Sandbox", state:ProjectState.PROJECT_STATE_ACTIVE, repositoryConnectionIds:[repositoryId], resourceVersion:"1", createdAt:now, updatedAt:now};
   certificateFixture(url.pathname);
   connectionFixture(url);
   profileFixture(url);
 }
 response.writeHead(200, { "content-type": "text/plain" }); response.end("ok"); });
health.listen(50062, "127.0.0.1");
function shutdown(): void { health.close(); grpc.tryShutdown(() => process.exit(0)); }
process.on("SIGTERM", shutdown);
process.on("SIGINT", shutdown);
