export interface PageDto {
  nextPageToken: string;
  snapshotCursor: string;
}

export interface PageInput {
  pageSize?: number;
  pageToken?: string;
}

export interface ResourceFailureDto {
  reason: string;
  message: string;
  remediation?: string;
  observedAt: string;
}

export interface OciPlatformDto {
  operatingSystem: string;
  architecture: string;
  variant?: string;
}

export type RepositoryConnectionStatusDto =
  | "connecting" | "connected" | "degraded" | "revoked" | "failed" | "unknown";

export interface GithubRepositoryDto {
  kind: "github";
  repositoryId: string;
  owner: string;
  name: string;
  installationId: string;
  htmlUrl: string;
}

export interface RepositoryConnectionDto {
  id: string;
  provider: GithubRepositoryDto;
  displayName: string;
  defaultBranch: string;
  status: RepositoryConnectionStatusDto;
  failure?: ResourceFailureDto;
  createdAt: string;
  updatedAt: string;
  resourceVersion: string;
}

export interface RepositoryConnectionPageDto {
  repositoryConnections: RepositoryConnectionDto[];
  page: PageDto;
}

export interface GithubInstallationOptionDto { installationId: string; owner: string; appName: string }
export interface GithubRepositoryOptionDto { repositoryId: string; owner: string; name: string; defaultBranch: string }

export interface CreateRepositoryConnectionInput {
  idempotencyKey: string;
  provider: GithubRepositoryDto;
  displayName: string;
  defaultBranch: string;
}

export type RuntimeProfileKindDto = "ayni_managed" | "custom_runtime" | "unknown";
export type RuntimeProfileSupportDto = "operational" | "unsupported" | "unknown";
export type RuntimeProfileStateDto =
  | "setup_required" | "setup_pr_open" | "awaiting_environment" | "ready"
  | "updating" | "degraded" | "failed" | "unknown";

export interface RuntimeProfileDto {
  id: string;
  repositoryConnectionId: string;
  kind: RuntimeProfileKindDto;
  support: RuntimeProfileSupportDto;
  state: RuntimeProfileStateDto;
  reconciliation: Array<{key:string;value:string}>;
  activeRevisionId?: string;
  activeRuntimeArtifactId?: string;
  failure?: ResourceFailureDto;
  createdAt: string;
  updatedAt: string;
  resourceVersion: string;
}

export interface RuntimeProfilePageDto {
  runtimeProfiles: RuntimeProfileDto[];
  page: PageDto;
}

export type RuntimeProfileConfigurationDto =
  | {
      kind: "ayni_managed";
      lockFingerprint: string;
      ayniVersion: string;
      workflowTemplateVersion: string;
      ociRepository: string;
    }
  | { kind: "custom_runtime"; imageReference: string };

export interface CreateRuntimeProfileInput {
  idempotencyKey: string;
  repositoryConnectionId: string;
  repositorySnapshotId: string;
  configuration: RuntimeProfileConfigurationDto;
}

export type RuntimeArtifactStatusDto =
  | "discovered" | "verifying" | "verified" | "rejected" | "unavailable" | "failed" | "unknown";

export interface RuntimeArtifactDto {
  id: string;
  runtimeProfileRevisionId: string;
  ociRepository: string;
  discoveryTag?: string;
  resolvedDigest: string;
  platform: OciPlatformDto;
  status: RuntimeArtifactStatusDto;
  verifiedMetadata: Array<{ key: string; value: string }>;
  failure?: ResourceFailureDto;
  createdAt: string;
  verifiedAt?: string;
  resourceVersion: string;
}

export interface RuntimeArtifactPageDto {
  runtimeArtifacts: RuntimeArtifactDto[];
  page: PageDto;
}

export interface ChangeConstraintsDto {
  protectedPaths: string[];
  allowTestEdits: boolean;
}

export interface AgentSelectionDto {
  provider: string;
  agent: string;
  model: string;
}

export interface DevelopmentTaskInputDto {
  repositoryConnectionId: string;
  repositoryRevision: string;
  runtimeProfileId: string;
  agent: AgentSelectionDto;
  request: string;
  acceptanceCriteria: string[];
  changeConstraints: ChangeConstraintsDto;
  deadlineSeconds?: number;
  publishDraft?: boolean;
}

export interface FrozenRepositoryDto {
  repositoryConnectionId: string;
  repositorySnapshotId: string;
  revision: string;
}

export interface FrozenRuntimeDto {
  runtimeProfileId: string;
  runtimeProfileRevisionId: string;
  profileKind: RuntimeProfileKindDto;
  runtimeArtifactId: string;
  ociRepository: string;
  resolvedDigest: string;
  platform: OciPlatformDto;
}

export interface DevelopmentSnapshotDto {
  repository: FrozenRepositoryDto;
  runtime: FrozenRuntimeDto;
  agent: AgentSelectionDto;
  request: string;
  acceptanceCriteria: string[];
  changeConstraints: ChangeConstraintsDto;
  deadlineAt?: string;
  publishDraft: boolean;
}

export type DevelopmentTaskStateDto =
  | "queued" | "waiting_for_runtime" | "running" | "succeeded" | "failed"
  | "cancelled" | "deadline_exceeded" | "unsupported" | "unknown";

export interface DevelopmentTaskDto {
  id: string;
  snapshot: DevelopmentSnapshotDto;
  state: DevelopmentTaskStateDto;
  activeAttemptId?: string;
  attemptCount: number;
  failure?: ResourceFailureDto;
  createdAt: string;
  updatedAt: string;
  resourceVersion: string;
}

export interface DevelopmentTaskPageDto {
  developmentTasks: DevelopmentTaskDto[];
  page: PageDto;
}

export type DevelopmentAttemptStateDto =
  | "queued"
  | "running"
  | "waiting_for_input"
  | "succeeded"
  | "failed"
  | "cancelled"
  | "deadline_exceeded"
  | "unsupported"
  | "unknown";

export interface DevelopmentAttemptDto {
  agentActivity: Array<{ sequence: number; observedAt?: string; message: string; failed: boolean }>;
  id: string;
  developmentTaskId: string;
  attemptNumber: number;
  snapshot: DevelopmentSnapshotDto;
  state: DevelopmentAttemptStateDto;
  requestedInput?: string;
  failure?: ResourceFailureDto;
  createdAt: string;
  updatedAt: string;
  completedAt?: string;
  resourceVersion: string;
}

export type DevelopmentTaskUpdateDto =
  | { kind: "development_task"; cursor: string; developmentTask: DevelopmentTaskDto }
  | { kind: "attempt"; cursor: string; attempt: DevelopmentAttemptDto }
  | { kind: "candidate"; cursor: string; candidateId: string };

export interface CreateDevelopmentTaskInput {
  idempotencyKey: string;
  request: DevelopmentTaskInputDto;
}

export interface TaskActionInput {
  idempotencyKey: string;
  developmentTaskId: string;
}

export interface ProviderModelDto {
  id: string;
  name?: string;
  ownedBy?: string;
  createdAt?: string;
  contextLength?: number;
  maxOutputTokens?: number;
}

export interface ProviderModelPageDto {
  provider: string;
  models: ProviderModelDto[];
  observedAt: string;
  page: PageDto;
}

export interface RequestContext {
  requestId: string;
  signal?: AbortSignal;
  deadlineMs?: number;
}
