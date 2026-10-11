import { mapCertificate } from "./certificates.js";
import { safeText, safeGithubLink, safeReconciliation } from "./onboarding.js";
import {
  QualityOutcome, workerPhaseToJSON, publicationOutcomeToJSON, executionOutcomeToJSON,
  type QualityResult, type PullRequestReceipt, type WorkerProgress, type WorkerResult,
  DevelopmentAttemptState,
  DevelopmentTaskState,
  ErrorReason,
  RepositoryConnectionStatus,
  RuntimeArtifactStatus,
  RuntimeProfileKind,
  RuntimeProfileState,
  RuntimeProfileSupport,
  type ChangeConstraints,
  type DevelopmentAttempt,
  type DevelopmentSnapshot,
  type DevelopmentTask,
  type DevelopmentTaskRequest,
  type OciPlatform,
  type ProviderModel,
  type RepositoryConnection,
  type ResourceFailure,
  type RuntimeArtifact,
  type RuntimeProfile,
  type RuntimeProfileConfiguration,
} from "@ayni-orchestrator/grpc-bindings/server";
import type {
  QualityResultDto, PullRequestReceiptDto, WorkerProgressDto, WorkerResultDto,
  ChangeConstraintsDto,
  CreateRepositoryConnectionInput,
  DevelopmentAttemptDto,
  DevelopmentAttemptStateDto,
  DevelopmentSnapshotDto,
  DevelopmentTaskDto,
  DevelopmentTaskInputDto,
  DevelopmentTaskStateDto,
  GithubRepositoryDto,
  OciPlatformDto,
  ProviderModelDto,
  RepositoryConnectionDto,
  RepositoryConnectionStatusDto,
  ResourceFailureDto,
  RuntimeArtifactDto,
  RuntimeArtifactStatusDto,
  RuntimeProfileConfigurationDto,
  RuntimeProfileDto,
  RuntimeProfileKindDto,
  RuntimeProfileStateDto,
  RuntimeProfileSupportDto,
} from "./dto.js";

interface DurationValue { seconds: string; nanos: number }

export class BffMappingError extends Error {
  constructor(message = "the control plane returned an incomplete response") {
    super(message);
    this.name = "BffMappingError";
  }
}

function required<T>(value: T | null | undefined): T {
  if (value === undefined || value === null) throw new BffMappingError();
  return value;
}

function optional<T extends object>(key: string, value: unknown): T | object {
  return value === undefined ? {} : { [key]: value };
}

function date(value: Date | undefined): string | undefined {
  if (value === undefined || !Number.isFinite(value.getTime())) return undefined;
  return value.toISOString();
}

function requiredDate(value: Date | undefined): string {
  return required(date(value));
}

function durationSeconds(value: DurationValue | undefined): number | undefined {
  if (value === undefined) return undefined;
  const seconds = Number(value.seconds) + value.nanos / 1_000_000_000;
  return Number.isFinite(seconds) && seconds >= 0 ? seconds : undefined;
}

function reason(value: ErrorReason): string {
  return ErrorReason[value]?.replace(/^ERROR_REASON_/u, "").toLowerCase() ?? "unknown";
}

function mapFailure(value: ResourceFailure | undefined): ResourceFailureDto | undefined {
  if (value === undefined) return undefined;
  return {
    reason: reason(value.reason),
    message: safeText(value.message,512),
    observedAt: requiredDate(value.observedAt),
    ...optional("remediation", value.remediation ? safeText(value.remediation,512) : undefined),
  } as ResourceFailureDto;
}

export function mapRepositoryStatus(value: RepositoryConnectionStatus): RepositoryConnectionStatusDto {
  switch (value) {
    case RepositoryConnectionStatus.REPOSITORY_CONNECTION_STATUS_CONNECTING: return "connecting";
    case RepositoryConnectionStatus.REPOSITORY_CONNECTION_STATUS_CONNECTED: return "connected";
    case RepositoryConnectionStatus.REPOSITORY_CONNECTION_STATUS_DEGRADED: return "degraded";
    case RepositoryConnectionStatus.REPOSITORY_CONNECTION_STATUS_REVOKED: return "revoked";
    case RepositoryConnectionStatus.REPOSITORY_CONNECTION_STATUS_FAILED: return "failed";
    default: return "unknown";
  }
}

export function mapRepositoryConnection(value: RepositoryConnection): RepositoryConnectionDto {
  if (value.provider?.provider?.$case !== "github") throw new BffMappingError();
  const github = value.provider.provider.github;
  return {
    id: value.id,
    provider: {
      kind: "github",
      repositoryId: github.repositoryId,
      owner: github.owner,
      name: github.name,
      installationId: github.installationId,
      htmlUrl: safeGithubLink(github.htmlUrl) ?? "",
    },
    displayName: safeText(value.displayName),
    useAyni: value.useAyni ?? true,
    onboardingStatus: safeText(value.onboardingStatus),
    onboardingReason: safeText(value.onboardingReason),
    onboardingRevision: safeText(value.onboardingRevision),
    registryConnectionId:value.registryConnectionId,
    imageNamespace:value.imageNamespace,
    defaultBranch: value.defaultBranch,
    status: mapRepositoryStatus(value.status),
    createdAt: requiredDate(value.createdAt),
    updatedAt: requiredDate(value.updatedAt),
    resourceVersion: value.resourceVersion,
    ...optional("failure", mapFailure(value.failure)),
  } as RepositoryConnectionDto;
}

export function toProtoRepositoryProvider(input: CreateRepositoryConnectionInput): {
  provider: { $case: "github"; github: Omit<GithubRepositoryDto, "kind"> };
} {
  const { kind: _kind, ...github } = input.provider;
  return { provider: { $case: "github", github } };
}

export function mapRuntimeProfileKind(value: RuntimeProfileKind): RuntimeProfileKindDto {
  switch (value) {
    case RuntimeProfileKind.RUNTIME_PROFILE_KIND_AYNI_MANAGED: return "ayni_managed";
    case RuntimeProfileKind.RUNTIME_PROFILE_KIND_CUSTOM_RUNTIME: return "custom_runtime";
    default: return "unknown";
  }
}

export function mapRuntimeProfileSupport(value: RuntimeProfileSupport): RuntimeProfileSupportDto {
  switch (value) {
    case RuntimeProfileSupport.RUNTIME_PROFILE_SUPPORT_OPERATIONAL: return "operational";
    case RuntimeProfileSupport.RUNTIME_PROFILE_SUPPORT_UNSUPPORTED: return "unsupported";
    default: return "unknown";
  }
}

export function mapRuntimeProfileState(value: RuntimeProfileState): RuntimeProfileStateDto {
  switch (value) {
    case RuntimeProfileState.RUNTIME_PROFILE_STATE_SETUP_REQUIRED: return "setup_required";
    case RuntimeProfileState.RUNTIME_PROFILE_STATE_SETUP_PR_OPEN: return "setup_pr_open";
    case RuntimeProfileState.RUNTIME_PROFILE_STATE_AWAITING_ENVIRONMENT: return "awaiting_environment";
    case RuntimeProfileState.RUNTIME_PROFILE_STATE_READY: return "ready";
    case RuntimeProfileState.RUNTIME_PROFILE_STATE_UPDATING: return "updating";
    case RuntimeProfileState.RUNTIME_PROFILE_STATE_DEGRADED: return "degraded";
    case RuntimeProfileState.RUNTIME_PROFILE_STATE_FAILED: return "failed";
    default: return "unknown";
  }
}

export function mapRuntimeProfile(value: RuntimeProfile): RuntimeProfileDto {
  return {
    id: value.id,
    repositoryConnectionId: value.repositoryConnectionId,
    kind: mapRuntimeProfileKind(value.kind),
    support: mapRuntimeProfileSupport(value.support),
    state: mapRuntimeProfileState(value.state),
    reconciliation: safeReconciliation(value.reconciliationMetadata),
    findings: value.findings.slice(0, 16).map(({ code, message, remediation }) => ({ code: safeText(code,128), message: safeText(message,512), remediation: safeText(remediation,512) })),
    preparationPhase: value.preparationPhase,
    rebuildAvailable: value.rebuildAvailable,
    desiredImageRepository: value.desiredImageRepository,
    ...optional("certificateId", value.certificateId),
    ...(value.effectiveCertificate ? { effectiveCertificate: mapCertificate(value.effectiveCertificate) } : {}),
    ...(value.registry === undefined ? {} : { registry: {registryConnectionId:value.registry.registryConnectionId} }),
    createdAt: requiredDate(value.createdAt),
    updatedAt: requiredDate(value.updatedAt),
    resourceVersion: value.resourceVersion,
    ...optional("activeRevisionId", value.activeRevisionId),
    ...optional("activeRuntimeArtifactId", value.activeRuntimeArtifactId),
    ...optional("failure", mapFailure(value.failure)),
  } as RuntimeProfileDto;
}

export function toProtoProfileConfiguration(value: RuntimeProfileConfigurationDto): RuntimeProfileConfiguration {
  return value.kind === "ayni_managed"
    ? {
        configuration: {
          $case: "ayniManaged",
          ayniManaged: {
            lockFingerprint: value.lockFingerprint,
            ayniVersion: value.ayniVersion,
            ociRepository: value.ociRepository,
          },
        },
      }
    : {
        configuration: {
          $case: "customRuntime",
          customRuntime: { imageReference: value.imageReference },
        },
      };
}

export function mapPlatform(value: OciPlatform): OciPlatformDto {
  return {
    operatingSystem: value.operatingSystem,
    architecture: value.architecture,
    ...optional("variant", value.variant),
  } as OciPlatformDto;
}

export function mapRuntimeArtifactStatus(value: RuntimeArtifactStatus): RuntimeArtifactStatusDto {
  switch (value) {
    case RuntimeArtifactStatus.RUNTIME_ARTIFACT_STATUS_DISCOVERED: return "discovered";
    case RuntimeArtifactStatus.RUNTIME_ARTIFACT_STATUS_VERIFYING: return "verifying";
    case RuntimeArtifactStatus.RUNTIME_ARTIFACT_STATUS_VERIFIED: return "verified";
    case RuntimeArtifactStatus.RUNTIME_ARTIFACT_STATUS_REJECTED: return "rejected";
    case RuntimeArtifactStatus.RUNTIME_ARTIFACT_STATUS_UNAVAILABLE: return "unavailable";
    case RuntimeArtifactStatus.RUNTIME_ARTIFACT_STATUS_FAILED: return "failed";
    default: return "unknown";
  }
}

export function mapRuntimeArtifact(value: RuntimeArtifact): RuntimeArtifactDto {
  return {
    id: value.id,
    runtimeProfileRevisionId: value.runtimeProfileRevisionId,
    ociRepository: value.ociRepository,
    resolvedDigest: value.resolvedDigest,
    platform: mapPlatform(required(value.platform)),
    status: mapRuntimeArtifactStatus(value.status),
    verifiedMetadata: value.verifiedMetadata.filter(item => ["certificate-key-id", "metadata-digest", "protected-content-root"].includes(item.key)).slice(0,3).map((item) => ({ key: item.key, value: safeText(item.value,256) })),
    createdAt: requiredDate(value.createdAt),
    resourceVersion: value.resourceVersion,
    ...optional("verifiedAt", date(value.verifiedAt)),
    ...optional("discoveryTag", value.discoveryTag),
    ...optional("failure", mapFailure(value.failure)),
  } as RuntimeArtifactDto;
}

export function mapTaskState(value: DevelopmentTaskState): DevelopmentTaskStateDto {
  switch (value) {
    case DevelopmentTaskState.DEVELOPMENT_TASK_STATE_QUEUED: return "queued";
    case DevelopmentTaskState.DEVELOPMENT_TASK_STATE_WAITING_FOR_RUNTIME: return "waiting_for_runtime";
    case DevelopmentTaskState.DEVELOPMENT_TASK_STATE_RUNNING: return "running";
    case DevelopmentTaskState.DEVELOPMENT_TASK_STATE_SUCCEEDED: return "succeeded";
    case DevelopmentTaskState.DEVELOPMENT_TASK_STATE_FAILED: return "failed";
    case DevelopmentTaskState.DEVELOPMENT_TASK_STATE_CANCELLED: return "cancelled";
    case DevelopmentTaskState.DEVELOPMENT_TASK_STATE_DEADLINE_EXCEEDED: return "deadline_exceeded";
    case DevelopmentTaskState.DEVELOPMENT_TASK_STATE_UNSUPPORTED: return "unsupported";
    default: return "unknown";
  }
}

export function mapChangeConstraints(value: ChangeConstraints): ChangeConstraintsDto {
  return { protectedPaths: value.protectedPaths.map(String), allowTestEdits: value.allowTestEdits };
}

export function mapDevelopmentSnapshot(value: DevelopmentSnapshot): DevelopmentSnapshotDto {
  const repository = required(value.repository);
  const runtime = required(value.runtime);
  const agent = required(value.agent);
  return {
    repository: {
      repositoryConnectionId: repository.repositoryConnectionId,
      repositorySnapshotId: repository.repositorySnapshotId,
      revision: repository.revision,
    },
    runtime: {
      runtimeProfileId: runtime.runtimeProfileId,
      runtimeProfileRevisionId: runtime.runtimeProfileRevisionId,
      profileKind: mapRuntimeProfileKind(runtime.profileKind),
      runtimeArtifactId: runtime.runtimeArtifactId,
      ociRepository: runtime.ociRepository,
      resolvedDigest: runtime.resolvedDigest,
      platform: mapPlatform(required(runtime.platform)),
    },
    agent: { provider: agent.provider, agent: agent.agent, model: agent.model },
    request: value.request,
    limits: { codingTurns: required(value.limits).codingTurns, qualityRepairLimit: required(value.limits).qualityRepairLimit, deadlineAt: date(value.limits?.deadlineAt) },
    acceptanceCriteria: value.acceptanceCriteria.map(String),
    changeConstraints: mapChangeConstraints(required(value.changeConstraints)),
    ...optional("deadlineAt", date(value.deadlineAt)),
  } as DevelopmentSnapshotDto;
}

export function mapDevelopmentTask(value: DevelopmentTask): DevelopmentTaskDto {
  return {
    projectId: value.projectId,
    id: value.id,
    snapshot: mapDevelopmentSnapshot(required(value.snapshot)),
    state: mapTaskState(value.state),
    attemptCount: value.attemptCount,
    createdAt: requiredDate(value.createdAt),
    updatedAt: requiredDate(value.updatedAt),
    resourceVersion: value.resourceVersion,
    ...optional("activeAttemptId", value.activeAttemptId),
    ...optional("failure", mapFailure(value.failure)),
  } as DevelopmentTaskDto;
}

export function mapAttemptState(value: DevelopmentAttemptState): DevelopmentAttemptStateDto {
  switch (value) {
    case DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_QUEUED: return "queued";
    case DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_RUNNING: return "running";
    case DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_WAITING_FOR_INPUT: return "waiting_for_input";
    case DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_SUCCEEDED: return "succeeded";
    case DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_FAILED: return "failed";
    case DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_CANCELLED: return "cancelled";
    case DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_DEADLINE_EXCEEDED: return "deadline_exceeded";
    case DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_UNSUPPORTED: return "unsupported";
    default: return "unknown";
  }
}

export function mapDevelopmentAttempt(value: DevelopmentAttempt): DevelopmentAttemptDto {
  return {
    progress: value.progress ? mapWorkerProgress(value.progress) : undefined,
    result: value.result ? mapWorkerResult(value.result) : undefined,
    pullRequest: value.pullRequest ? mapPullRequest(value.pullRequest) : undefined,
    publicationOnly: value.publicationOnly,
    agentActivity: (value.agentActivity ?? []).slice(-256).map(event => ({
      sequence: event.sequence, observedAt: date(event.observedAt),
      message: event.message.slice(0, 256), failed: event.failed,
    })),
    id: value.id,
    developmentTaskId: value.developmentTaskId,
    attemptNumber: value.attemptNumber,
    snapshot: mapDevelopmentSnapshot(required(value.snapshot)),
    state: mapAttemptState(value.state),
    createdAt: requiredDate(value.createdAt),
    updatedAt: requiredDate(value.updatedAt),
    resourceVersion: value.resourceVersion,
    ...optional("requestedInput", value.requestedInput),
    ...optional("completedAt", date(value.completedAt)),
    ...optional("failure", mapFailure(value.failure)),
  } as DevelopmentAttemptDto;
}

export function toProtoTaskRequest(value: DevelopmentTaskInputDto): DevelopmentTaskRequest {
  const deadlineSeconds = value.deadlineSeconds;
  return {
    projectId: value.projectId,
    repositoryConnectionId: value.repositoryConnectionId,
    repositoryRevision: value.repositoryRevision,
    runtimeProfileId: value.runtimeProfileId,
    agent: { ...value.agent },
    request: value.request,
    acceptanceCriteria: value.acceptanceCriteria.map(String),
    changeConstraints: {
      protectedPaths: value.changeConstraints.protectedPaths.map(String),
      allowTestEdits: value.changeConstraints.allowTestEdits,
    },
    codingTurns: value.codingTurns,
    ...(deadlineSeconds === undefined
      ? {}
      : { deadline: { seconds: String(deadlineSeconds), nanos: 0 } }),
  };
}

export function mapProviderModel(value: ProviderModel): ProviderModelDto {
  return {
    id: value.id,
    ...optional("name", value.name),
    ...optional("ownedBy", value.ownedBy),
    ...optional("createdAt", date(value.createdAt)),
    ...optional("contextLength", value.contextLength),
    ...optional("maxOutputTokens", value.maxOutputTokens),
  } as ProviderModelDto;
}

export function mapQuality(value: QualityResult): QualityResultDto {
  const evidence = value.evidence;
  const validEvidence = evidence && /^(?:sha256:)?[a-f0-9]{64}$/u.test(evidence.signalsDigest) && /^(?:sha256:)?[a-f0-9]{64}$/u.test(evidence.sourceFingerprint) && /^[a-f0-9]{40}$/u.test(evidence.commit);
  const outcome = value.outcome === QualityOutcome.QUALITY_OUTCOME_FAILED ? "failed" : value.outcome === QualityOutcome.QUALITY_OUTCOME_PASSED && validEvidence ? "passed" : "unavailable";
  return { outcome, completedTargets: value.completedTargets, skippedTargets: value.skippedTargets, warningOffenders: value.warningOffenders, failingOffenders: value.failingOffenders,
    rows: value.rows.slice(0, 64).map(row => ({ kind: safeText(row.kind, 128), passed: row.passed, status: !row.passed ? "fail" : row.status === "warn" ? "warn" : "pass", summary: safeText(row.summary, 1024), target: safeText(row.target, 256) })), findings: value.findings.slice(0, 32).map(finding => safeText(finding, 1024)),
    evidence: validEvidence ? { signalsDigest: evidence.signalsDigest, sourceFingerprint: evidence.sourceFingerprint, commit: evidence.commit } : undefined };
}
export function mapPullRequest(value: PullRequestReceipt): PullRequestReceiptDto {
  return { repository: safeText(value.repository, 256), number: value.number, url: safeGithubLink(value.url), branch: safeText(value.branch, 256), baseBranch: safeText(value.baseBranch, 256), headCommit: safeText(value.headCommit, 40), draft: value.draft };
}
export function mapWorkerProgress(value: WorkerProgress): WorkerProgressDto {
  return { phase: workerPhaseToJSON(value.phase).replace("WORKER_PHASE_", "").toLowerCase(), codingTurnsConsumed: value.codingTurnsConsumed, qualityRound: value.qualityRound,
    latestFullCheck: value.latestFullCheck ? mapQuality(value.latestFullCheck) : undefined, publication: publicationOutcomeToJSON(value.publication).replace("PUBLICATION_OUTCOME_", "").toLowerCase() };
}
export function mapWorkerResult(value: WorkerResult): WorkerResultDto {
  return { execution: executionOutcomeToJSON(value.execution).replace("EXECUTION_OUTCOME_", "").toLowerCase(), commit: value.commit, changedPaths: value.changedPaths.slice(0, 256).map(path => safeText(path, 1024)),
    quality: value.quality ? mapQuality(value.quality) : undefined, requestedInput: value.requestedInput ? safeText(value.requestedInput, 1024) : undefined,
    publication: publicationOutcomeToJSON(value.publication).replace("PUBLICATION_OUTCOME_", "").toLowerCase(), pullRequest: value.pullRequest ? mapPullRequest(value.pullRequest) : undefined,
    codingTurnsConsumed: value.codingTurnsConsumed, qualityRound: value.qualityRound };
}
