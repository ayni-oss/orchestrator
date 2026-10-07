import { createServer } from "node:http";
import { Server, ServerCredentials, status, type UntypedServiceImplementation } from "@grpc/grpc-js";
import {
  ConnectionServiceService, type Connection,
  ProjectServiceService, ProjectState,
  DevelopmentAttemptState,
  AutomationInstallationState,
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
  status: RepositoryConnectionStatus.REPOSITORY_CONNECTION_STATUS_CONNECTED,
  createdAt: now,
  updatedAt: now,
  resourceVersion: "1",
};

const secondRepository = {...repository,id:"repository:2",displayName:"ayni-oss/library",provider:{provider:{$case:"github",github:{...repository.provider.provider.github,repositoryId:"456",name:"library",htmlUrl:"https://github.com/ayni-oss/library"}}}};
let project: any = {id:"project:1", displayName:"Sandbox", state:ProjectState.PROJECT_STATE_ACTIVE, repositoryConnectionIds:[repositoryId], resourceVersion:"1", createdAt:now, updatedAt:now};
const profile: any = {
  id: profileId,
  repositoryConnectionId: repositoryId,
  kind: RuntimeProfileKind.RUNTIME_PROFILE_KIND_AYNI_MANAGED,
  support: RuntimeProfileSupport.RUNTIME_PROFILE_SUPPORT_OPERATIONAL,
  state: RuntimeProfileState.RUNTIME_PROFILE_STATE_READY,
  activeRevisionId: "profile-revision:1",
  activeRuntimeArtifactId: "artifact:1",
  reconciliationMetadata: [],
  createdAt: now,
  updatedAt: now,
  resourceVersion: "2",
};

function snapshot(request = "Correct the bounded retry formula", sourceRevision = revision, model = "gpt-5"): any {
  return {
    limits: {codingTurns:24,qualityRepairLimit:3},
    repository: { repositoryConnectionId: repositoryId, repositorySnapshotId: "snapshot:1", revision: sourceRevision },
    runtime: {
      runtimeProfileId: profileId,
      runtimeProfileRevisionId: "profile-revision:1",
      profileKind: RuntimeProfileKind.RUNTIME_PROFILE_KIND_AYNI_MANAGED,
      runtimeArtifactId: "artifact:1",
      ociRepository: "ghcr.io/ayni-oss/sandbox/ayni-environment",
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
  listGithubInstallations(_call, callback) { callback(null, { installations: [{ installationId: "42", owner: "ayni-oss", appName: "Ayni" }] }); },
  listGithubInstallationRepositories(_call, callback) { callback(null, { repositories: [{ repositoryId: "123", owner: "ayni-oss", name: "sandbox", defaultBranch: "main" }] }); },
  setRepositoryRegistry(call,callback){repository={...repository,registryConnectionId:call.request.registryConnectionId,imageNamespace:call.request.imageNamespace,resourceVersion:String(version++)};callback(null,{repositoryConnection:repository});},
  connectGithubRepository(_call,callback){callback(null,{repositoryConnection:repository});},

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
  const connection=connections.find(item=>item.id===profile.registry?.registryConnectionId);
  const imageName=connection?.imageNamespace.toLowerCase()==="ayni-oss" ? "sandbox-ayni" : "ayni-oss/sandbox-ayni";
  return {...profile,desiredImageRepository:connection ? `ghcr.io/${connection.imageNamespace}/${imageName}` : "ghcr.io/ayni-oss/sandbox-ayni"};
}

let installation:any;
let profileEnrolled = true;
const runtimeProfileService: UntypedServiceImplementation = {
  inspectAyniRepository(_call,callback){callback(null,{repositorySnapshotId:"snapshot:1",repositoryRevision:revision,lockFingerprint:"c".repeat(64),ayniVersion:"0.14.5",workflowTemplateVersion:"ayni-managed-v3",ociRepository:"ghcr.io/ayni-oss/sandbox-ayni",policyPresent:true,lockCompatible:true,trustPolicyPinned:true,reason:"signing_configuration_requires_operator",workflows:[{path:".github/workflows/ayni-environment.yml",contents:"name: Build signed environment"}]});},
  createRuntimeProfile(_call,callback){profileEnrolled=true;profile.state=RuntimeProfileState.RUNTIME_PROFILE_STATE_SETUP_REQUIRED;callback(null,{runtimeProfile:profileView()});},
  installRepositoryAutomation(_call,callback){profile.state=RuntimeProfileState.RUNTIME_PROFILE_STATE_SETUP_PR_OPEN;installation={id:"automation:1",runtimeProfileId:profileId,version:"ayni-managed-v3",kind:1,state:AutomationInstallationState.AUTOMATION_INSTALLATION_STATE_SETUP_PR_OPEN,provider:{$case:"github",github:{installationId:"42",setupPullRequestNumber:"8",environmentWorkflowPath:".github/workflows/ayni-environment.yml"}},resourceVersion:"1"};callback(null,{automationInstallation:installation});},
  setRuntimeRegistry(call,callback){
    if(call.request.expectedResourceVersion!==profile.resourceVersion) return callback({code:status.ABORTED,message:"Refresh the runtime and try again."});
    const connection=connections.find(item=>item.id===call.request.registryConnectionId);
    if(!connection || connection.kind!=="ghcr" || !connection.enabled || connection.status!=="ready") return callback({code:status.FAILED_PRECONDITION,message:"Choose a ready GHCR connection."});
    profile.registry={registryConnectionId:connection.id};
    profile.resourceVersion=String(Number(profile.resourceVersion)+1);
    callback(null,{runtimeProfile:profileView()});
  },
  reconcileRuntimeProfile(_call,callback){profile.resourceVersion=String(Number(profile.resourceVersion)+1);profile.state=RuntimeProfileState.RUNTIME_PROFILE_STATE_READY;callback(null,{runtimeProfile:profileView()});},

  getRuntimeProfileRevision(_call,callback) { callback(null,{revision:{id:"profile-revision:1",runtimeProfileId:profileId,repositoryRevision:revision,repositorySnapshotId:"snapshot:1",revisionNumber:"1",resourceVersion:"1",configuration:{$case:"ayniManaged",ayniManaged:{lockFingerprint:"c".repeat(64),ayniVersion:"0.14.5",workflowTemplateVersion:"ayni-managed-v3",ociRepository:"ghcr.io/ayni-oss/sandbox-ayni"}}}}); },
  listAutomationInstallations(_call,callback) { callback(null,{automationInstallations:installation ? [installation] : [],page}); },

  listRuntimeProfiles(call, callback) {
    const profiles = profileEnrolled && (call.request.repositoryConnectionId === undefined || call.request.repositoryConnectionId === repositoryId) ? [profileView()] : [];
    callback(null, { runtimeProfiles: profiles, page });
  },
  getRuntimeProfile(_call, callback) { callback(null, { runtimeProfile: profileView() }); },
  listRuntimeArtifacts(_call, callback) { callback(null, { runtimeArtifacts: [], page }); },
};

const providerService: UntypedServiceImplementation = {
  listProviderModels(_call, callback) { callback(null, { provider: "openai", models: [{ id: "gpt-5", name: "GPT-5", ownedBy: "openai", contextLength: "400000", maxOutputTokens: "128000" }], observedAt: now, page }); },
};

const taskService: UntypedServiceImplementation = {
  listDevelopmentAttempts(call, callback) {
    const t = tasks.get(call.request.developmentTaskId);
    const publicationFailed = t?.id.endsWith("0044");
    callback(null, { attempts: t ? [{publicationOnly:publicationRetries.has(t.id), id: `attempt:${t.id}`, developmentTaskId:t.id, attemptNumber:1, snapshot:t.snapshot, state:DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_SUCCEEDED, agentActivity:[], progress:{phase:8,codingTurnsConsumed:5,qualityRound:2,publication:3}, result:{execution:1,commit:"d".repeat(40),changedPaths:["src/main.rs"],quality:{outcome:2,completedTargets:1,skippedTargets:0,warningOffenders:0,failingOffenders:1,rows:[{kind:"test",passed:false}],findings:["Repair the failing test"],evidence:{artifactPath:".ayni/last/signals.json",signalsDigest:"e".repeat(64),sourceFingerprint:"f".repeat(64),commit:"d".repeat(40)}},publication:publicationFailed ? 4 : 3,codingTurnsConsumed:5,qualityRound:2,pullRequest:publicationFailed ? undefined : {repository:"ayni-oss/sandbox",number:"9",url:"https://github.com/ayni-oss/sandbox/pull/9",branch:"task/example",baseBranch:"main",headCommit:"d".repeat(40),draft:true}},createdAt:now,updatedAt:now,resourceVersion:"1"}] : [], page});
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
      limits:{codingTurns:24,qualityRepairLimit:3},
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
grpc.addService(ConnectionServiceService,connectionService);
grpc.addService(ProjectServiceService, {
  listProjects(_call:any,callback:any){callback(null,{projects:[project],page});},
  getProject(_call:any,callback:any){callback(null,{project});},
  createProject(call:any,callback:any){project={...project,displayName:call.request.displayName,repositoryConnectionIds:call.request.repositoryConnectionIds.length ? call.request.repositoryConnectionIds : [call.request.repositoryConnectionId],state:ProjectState.PROJECT_STATE_ACTIVE,resourceVersion:"1"};callback(null,{project});},
  updateProject(call:any,callback:any){project={...project,displayName:call.request.displayName,repositoryConnectionIds:call.request.repositoryConnectionIds.length ? call.request.repositoryConnectionIds : project.repositoryConnectionIds,resourceVersion:String(Number(project.resourceVersion)+1)};callback(null,{project});},
  archiveProject(_call:any,callback:any){project={...project,state:ProjectState.PROJECT_STATE_ARCHIVED,resourceVersion:String(Number(project.resourceVersion)+1)};callback(null,{project});},
});
grpc.addService(RepositoryServiceService, repositoryService);
grpc.addService(RuntimeProfileServiceService, runtimeProfileService);
grpc.addService(ProviderServiceService, providerService);
grpc.addService(DevelopmentTaskServiceService, taskService);
grpc.bindAsync("127.0.0.1:50061", ServerCredentials.createInsecure(), (error) => { if (error) throw error; });
const health = createServer((request, response) => {
 const url=new URL(request.url ?? "/","http://127.0.0.1");
 if(request.method==="POST" && url.pathname==="/reset-connections") {connections=[];rejectConnectionMutation=false;}
 if(request.method==="POST" && url.pathname==="/reject-connection-mutation") rejectConnectionMutation=true;
 if(request.method==="POST" && url.pathname==="/connection-state" && connections[0]) {
   connections[0].resourceVersion=String(Number(connections[0].resourceVersion)+1);
   connections[0].status=url.searchParams.get("status") ?? "ready";
   connections[0].testCleanup=url.searchParams.get("cleanup") ?? "passed";
   connections[0].testCleanupFailureReason=connections[0].testCleanup==="failed" ? "registry_cleanup_permission_required" : "";
 }
 if(request.method==="POST" && url.pathname==="/admin-connection") connections=[{id:"admin:github",kind:url.searchParams.get("kind") ?? "github_app",managementSource:"admin",displayName:"Admin App",appId:"123",username:"",imageNamespace:url.searchParams.get("kind")==="ghcr" ? "admin-team" : "",secretName:"admin-ref",secretUid:"",desiredRevision:"1",observedRevision:"1",status:"ready",failureReason:"",enabled:true,resourceVersion:"1",pullSecretName:"",testAuthentication:"",testPush:"",testPull:"",testCleanup:"",testArtifactDigest:"",candidateSecretName:"",candidateSecretUid:"",candidatePullSecretName:"",candidatePullSecretUid:"",pullSecretUid:"",retiredSecretRevisions:[],testCleanupFailureReason:"",testArtifactRepository:""}];
 if(request.method==="POST" && url.pathname==="/reset-onboarding") { profileEnrolled=false; installation=undefined; }
 if(request.method==="POST" && url.pathname==="/profile-version") profile.resourceVersion=String(Number(profile.resourceVersion)+1);
 if(request.method==="POST" && url.pathname==="/profile-state") { const state=Number(url.searchParams.get("value")); if(Number.isInteger(state) && state>=1 && state<=7) profile.state=state; }
 response.writeHead(200, { "content-type": "text/plain" }); response.end("ok"); });
health.listen(50062, "127.0.0.1");
function shutdown(): void { health.close(); grpc.tryShutdown(() => process.exit(0)); }
process.on("SIGTERM", shutdown);
process.on("SIGINT", shutdown);
