import {
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
    message: value.message,
    observedAt: requiredDate(value.observedAt),
    ...optional("remediation", value.remediation),
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
      htmlUrl: github.htmlUrl,
    },
    displayName: value.displayName,
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
            workflowTemplateVersion: value.workflowTemplateVersion,
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
    verifiedMetadata: value.verifiedMetadata.map((item) => ({ key: item.key, value: item.value })),
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
    acceptanceCriteria: value.acceptanceCriteria.map(String),
    changeConstraints: mapChangeConstraints(required(value.changeConstraints)),
    ...optional("deadlineAt", date(value.deadlineAt)),
  } as DevelopmentSnapshotDto;
}

export function mapDevelopmentTask(value: DevelopmentTask): DevelopmentTaskDto {
  return {
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
