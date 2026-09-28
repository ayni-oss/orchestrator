import { createServer } from "node:http";
import { Server, ServerCredentials, type UntypedServiceImplementation } from "@grpc/grpc-js";
import {
  DevelopmentTaskPhase,
  DevelopmentTaskServiceService,
  EvidenceOutcome,
  LifecycleOutcome,
  LifecycleStage,
  ModelInteractionOutcome,
  ModelUsageCollectionStatus,
  ModelUsageStage,
  PreparationStatus,
  ProviderServiceService,
  RepositoryServiceService,
} from "@ayni-orchestrator/grpc-bindings/server";

const taskId = "00000000-0000-4000-8000-000000000039";
const revision = "a".repeat(40);
const lock = "b".repeat(64);
const now = new Date("2026-09-28T10:00:00.000Z");
let version = 1;

const repository = {
  enrollment: { id: "sandbox", owner: "ayni-oss", repository: "sandbox", installationId: "42", baseBranch: "main", lockSha256: lock },
  latestPreparation: { id: "prep-sandbox", repositoryId: "sandbox", revision, lockSha256: lock, status: PreparationStatus.PREPARATION_STATUS_READY, duration: { seconds: "48", nanos: 0 }, reuseCount: 1 },
};

const tasks = new Map<string, any>();
function task(id = taskId, phase = DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_WAITING_FOR_INPUT, request = "Correct the bounded retry formula"): any {
  return {
    id,
    input: { repositoryId: "sandbox", model: "gpt-5", startRevision: revision, request, acceptanceCriteria: ["Three retries produce eight attempts", "Existing checks remain green"], changeConstraints: { protectedPaths: [".github/workflows"], allowTestEdits: false }, deadline: { seconds: "3600", nanos: 0 } },
    repository: { id: "sandbox", owner: "ayni-oss", repository: "sandbox", baseBranch: "main", lockSha256: lock },
    model: { provider: "openai", model: "gpt-5" },
    preparationId: "prep-sandbox",
    createdAt: now,
    updatedAt: now,
    deadlineAt: new Date("2026-09-28T11:00:00.000Z"),
    phase,
    ...(phase === DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_WAITING_FOR_INPUT ? { requestedInput: "Which retry path should remain backward compatible?" } : {}),
    scopeViolations: [],
    lifecycleComplete: [DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_COMPLETED, DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_CANCELLED].includes(phase),
    lifecycle: [
      { sequence: "1", eventTime: now, stage: LifecycleStage.LIFECYCLE_STAGE_QUEUE, outcome: LifecycleOutcome.LIFECYCLE_OUTCOME_SUCCEEDED },
      { sequence: "2", eventTime: now, stage: LifecycleStage.LIFECYCLE_STAGE_CODING, outcome: phase === DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_WAITING_FOR_INPUT ? LifecycleOutcome.LIFECYCLE_OUTCOME_BLOCKED : LifecycleOutcome.LIFECYCLE_OUTCOME_STARTED, reasonCode: phase === DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_WAITING_FOR_INPUT ? "operator_input_required" : undefined },
    ],
    modelUsage: {
      collectionStatus: ModelUsageCollectionStatus.MODEL_USAGE_COLLECTION_STATUS_COLLECTING,
      interactions: [{ callId: "call-1", stage: ModelUsageStage.MODEL_USAGE_STAGE_CODING, provider: "openai", model: "gpt-5", startedAt: now, completedAt: now, latency: { seconds: "2", nanos: 0 }, outcome: ModelInteractionOutcome.MODEL_INTERACTION_OUTCOME_SUCCEEDED, tokens: { inputTokens: "120", outputTokens: "40", totalTokens: "160" }, estimatedCost: { label: "Estimated provider cost", amountMicros: "3500", currency: "USD", pricingVersion: "fixture-v1", pricedInputTokens: "120", pricedCachedInputTokens: "0", pricedOutputTokens: "40", inputRateMicrosPerMillion: "10000000", cachedInputRateMicrosPerMillion: "0", outputRateMicrosPerMillion: "20000000" } }],
      byStage: [{ stage: ModelUsageStage.MODEL_USAGE_STAGE_CODING, summary: { interactionCount: 1, unavailableInteractionCount: 0, inputTokens: "120", outputTokens: "40", totalTokens: "160", estimatedCostMicros: "3500", estimatedCostCurrency: "USD", pricingVersion: "fixture-v1" } }],
      total: { interactionCount: 1, unavailableInteractionCount: 0, inputTokens: "120", outputTokens: "40", totalTokens: "160", estimatedCostMicros: "3500", estimatedCostCurrency: "USD", pricingVersion: "fixture-v1" },
    },
    resourceVersion: String(version++),
  };
}
tasks.set(taskId, task());
tasks.set("00000000-0000-4000-8000-000000000043", task("00000000-0000-4000-8000-000000000043", DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_CODING, "Active task for cancellation"));

function summary(value: any): any {
  return { id: value.id, repositoryId: value.repository.id, repositoryOwner: value.repository.owner, repositoryName: value.repository.repository, startRevision: value.input.startRevision, request: value.input.request, model: value.model, retryOfTaskId: value.retryOfTaskId, createdAt: value.createdAt, updatedAt: value.updatedAt, phase: value.phase, draftPrUrl: value.draftPrUrl, failure: value.failure, modelUsage: value.modelUsage.total, resourceVersion: value.resourceVersion };
}

const repositoryService: UntypedServiceImplementation = {
  listRepositories(_call, callback) { callback(null, { repositories: [repository], page: { nextPageToken: "" } }); },
  getRepository(_call, callback) { callback(null, { repository }); },
  saveRepository(call, callback) { repository.enrollment = call.request.repository; callback(null, { repository }); },
  listPreparations(_call, callback) { callback(null, { preparations: [repository.latestPreparation], page: { nextPageToken: "" } }); },
  getPreparation(_call, callback) { callback(null, { preparation: repository.latestPreparation }); },
  prepareRepository(call, callback) { repository.latestPreparation = { ...repository.latestPreparation, revision: call.request.revision, reuseCount: repository.latestPreparation.reuseCount + 1 }; callback(null, { preparation: repository.latestPreparation, reused: true }); },
};

const providerService: UntypedServiceImplementation = {
  listProviderModels(_call, callback) { callback(null, { provider: "openai", models: [{ id: "gpt-5", name: "GPT-5", ownedBy: "openai", contextLength: "400000", maxOutputTokens: "128000" }], observedAt: now, page: { nextPageToken: "" } }); },
};

const taskService: UntypedServiceImplementation = {
  listDevelopmentTasks(call, callback) { const values = [...tasks.values()].filter((value) => call.request.phase === undefined || value.phase === call.request.phase); callback(null, { tasks: values.map(summary), page: { nextPageToken: "" } }); },
  getDevelopmentTask(call, callback) { callback(null, { task: tasks.get(call.request.taskId) ?? tasks.get(taskId) }); },
  createDevelopmentTask(call, callback) { const id = "00000000-0000-4000-8000-000000000040"; const created = task(id, DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_QUEUED, call.request.input.request); created.input = call.request.input; tasks.set(id, created); callback(null, { task: created }); },
  cancelDevelopmentTask(call, callback) { const value = tasks.get(call.request.taskId); value.phase = DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_CANCELLED; value.lifecycleComplete = true; value.updatedAt = new Date(); value.resourceVersion = String(version++); callback(null, { task: value }); },
  retryDevelopmentTask(call, callback) { const source = tasks.get(call.request.taskId); const id = "00000000-0000-4000-8000-000000000041"; const retried = task(id, DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_QUEUED, source.input.request); retried.retryOfTaskId = source.id; tasks.set(id, retried); callback(null, { task: retried }); },
  provideDevelopmentTaskInput(call, callback) { const source = tasks.get(call.request.taskId); const id = "00000000-0000-4000-8000-000000000042"; const followup = task(id, DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_CODING, source.input.request); followup.retryOfTaskId = source.id; followup.providedInput = call.request.input; tasks.set(id, followup); callback(null, { task: followup }); },
  getDevelopmentTaskEvidence(call, callback) { const value = tasks.get(call.request.taskId); if (value.phase !== DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_COMPLETED) { callback({ code: 5, message: "evidence unavailable" }); return; } callback(null, { evidence: { manifestDigest: `sha256:${"c".repeat(64)}`, outcome: EvidenceOutcome.EVIDENCE_OUTCOME_SUCCESS, checkExitCode: 0, startRevision: revision, candidateCommit: "d".repeat(40), lockSha256: lock, changeConstraints: value.input.changeConstraints, preparationId: "prep-sandbox", preparationReused: true, policyEdited: false, testsEdited: false, scopeViolations: [], changedPaths: ["src/lib.rs"], artifacts: [{ name: "execution.json", sha256: "e".repeat(64) }] } }); },
  watchDevelopmentTask(call) { const value = tasks.get(call.request.taskId); if (value !== undefined) call.write({ task: value, resourceVersion: value.resourceVersion }); call.end(); },
};

const grpc = new Server();
grpc.addService(RepositoryServiceService, repositoryService);
grpc.addService(ProviderServiceService, providerService);
grpc.addService(DevelopmentTaskServiceService, taskService);
grpc.bindAsync("127.0.0.1:50061", ServerCredentials.createInsecure(), (error) => { if (error) throw error; });
const health = createServer((_request, response) => { response.writeHead(200, { "content-type": "text/plain" }); response.end("ok"); });
health.listen(50062, "127.0.0.1");

function shutdown(): void {
  health.close();
  grpc.tryShutdown(() => process.exit(0));
}
process.on("SIGTERM", shutdown);
process.on("SIGINT", shutdown);
