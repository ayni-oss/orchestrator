import { createServer } from "node:http";
import { Server, ServerCredentials, type UntypedServiceImplementation } from "@grpc/grpc-js";
import {
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
  status: RepositoryConnectionStatus.REPOSITORY_CONNECTION_STATUS_CONNECTED,
  createdAt: now,
  updatedAt: now,
  resourceVersion: "1",
};

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
tasks.set("00000000-0000-4000-8000-000000000043", task("00000000-0000-4000-8000-000000000043", DevelopmentTaskState.DEVELOPMENT_TASK_STATE_RUNNING, "Active task for cancellation"));

const page = { nextPageToken: "", snapshotCursor: "10" };
const repositoryService: UntypedServiceImplementation = {
  listRepositoryConnections(_call, callback) { callback(null, { repositoryConnections: [repository], page }); },
  getRepositoryConnection(_call, callback) { callback(null, { repositoryConnection: repository }); },
  createRepositoryConnection(call, callback) {
    repository = { ...repository, provider: call.request.provider, displayName: call.request.displayName, defaultBranch: call.request.defaultBranch, updatedAt: new Date(), resourceVersion: String(version++) };
    callback(null, { repositoryConnection: repository });
  },
  listRepositorySnapshots(_call, callback) { callback(null, { repositorySnapshots: [{ id: "snapshot:1", repositoryConnectionId: repositoryId, revision, defaultBranch: "main", observedAt: now, resourceVersion: "3" }], page }); },
  getRepositorySnapshot(_call, callback) { callback(null, { repositorySnapshot: { id: "snapshot:1", repositoryConnectionId: repositoryId, revision, defaultBranch: "main", observedAt: now, resourceVersion: "3" } }); },
  watchRepositories(call) { call.write({ cursor: "10", update: { $case: "repositoryConnection", repositoryConnection: repository } }); call.end(); },
};

const runtimeProfileService: UntypedServiceImplementation = {
  listRuntimeProfiles(call, callback) {
    const profiles = call.request.repositoryConnectionId === undefined || call.request.repositoryConnectionId === repositoryId ? [profile] : [];
    callback(null, { runtimeProfiles: profiles, page });
  },
  getRuntimeProfile(_call, callback) { callback(null, { runtimeProfile: profile }); },
  listRuntimeArtifacts(_call, callback) { callback(null, { runtimeArtifacts: [], page }); },
};

const providerService: UntypedServiceImplementation = {
  listProviderModels(_call, callback) { callback(null, { provider: "openai", models: [{ id: "gpt-5", name: "GPT-5", ownedBy: "openai", contextLength: "400000", maxOutputTokens: "128000" }], observedAt: now, page }); },
};

const taskService: UntypedServiceImplementation = {
  listDevelopmentTasks(_call, callback) { callback(null, { developmentTasks: [...tasks.values()], page }); },
  getDevelopmentTask(call, callback) { callback(null, { developmentTask: tasks.get(call.request.developmentTaskId) ?? tasks.get(taskId) }); },
  createDevelopmentTask(call, callback) {
    const id = "00000000-0000-4000-8000-000000000040";
    const request = call.request.request;
    const created = task(id, DevelopmentTaskState.DEVELOPMENT_TASK_STATE_QUEUED, request.request);
    created.snapshot = {
      ...snapshot(request.request, request.repositoryRevision, request.agent.model),
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
grpc.addService(RepositoryServiceService, repositoryService);
grpc.addService(RuntimeProfileServiceService, runtimeProfileService);
grpc.addService(ProviderServiceService, providerService);
grpc.addService(DevelopmentTaskServiceService, taskService);
grpc.bindAsync("127.0.0.1:50061", ServerCredentials.createInsecure(), (error) => { if (error) throw error; });
const health = createServer((_request, response) => { response.writeHead(200, { "content-type": "text/plain" }); response.end("ok"); });
health.listen(50062, "127.0.0.1");
function shutdown(): void { health.close(); grpc.tryShutdown(() => process.exit(0)); }
process.on("SIGTERM", shutdown);
process.on("SIGINT", shutdown);
