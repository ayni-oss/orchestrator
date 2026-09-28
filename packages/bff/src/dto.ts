export interface PageDto {
  nextPageToken: string;
}

export interface PageInput {
  pageSize?: number;
  pageToken?: string;
}

export interface ChangeConstraintsDto {
  protectedPaths: string[];
  allowTestEdits: boolean;
}

export type PreparationStatusDto =
  | "queued"
  | "preparing"
  | "ready"
  | "setup_required"
  | "failed"
  | "unknown";

export interface PreparationDto {
  id: string;
  repositoryId: string;
  revision: string;
  lockSha256: string;
  status: PreparationStatusDto;
  diagnostic?: string;
  durationSeconds?: number;
  reuseCount: number;
}

export interface RepositoryEnrollmentDto {
  id: string;
  owner: string;
  repository: string;
  installationId: string;
  baseBranch: string;
  lockSha256: string;
}

export interface RepositoryDto {
  enrollment: RepositoryEnrollmentDto;
  latestPreparation?: PreparationDto;
}

export interface RepositoryPageDto {
  repositories: RepositoryDto[];
  page: PageDto;
}

export interface PreparationPageDto {
  preparations: PreparationDto[];
  page: PageDto;
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

export type DevelopmentTaskPhaseDto =
  | "queued"
  | "coding"
  | "waiting_for_input"
  | "validation_queued"
  | "validating"
  | "pr_queued"
  | "preparing_pr"
  | "completed"
  | "failed"
  | "cancelled"
  | "unsupported"
  | "unknown";

export type LifecycleStageDto = "queue" | "coding" | "validation" | "pr_preparation" | "unknown";
export type LifecycleOutcomeDto =
  | "queued"
  | "started"
  | "succeeded"
  | "blocked"
  | "failed"
  | "cancelled"
  | "unsupported"
  | "unknown";
export type ModelUsageStageDto = "coding" | "pr_preparation" | "unknown";
export type ModelInteractionOutcomeDto = "succeeded" | "failed" | "cancelled" | "unknown";
export type ModelUsageCollectionStatusDto = "unavailable" | "collecting" | "complete" | "unknown";
export type EvidenceOutcomeDto =
  | "success"
  | "quality_failure"
  | "policy_rejected"
  | "dependency_change_unsupported"
  | "scope_rejected"
  | "unknown";

export interface DevelopmentTaskInputDto {
  repositoryId: string;
  model: string;
  startRevision: string;
  request: string;
  acceptanceCriteria: string[];
  changeConstraints: ChangeConstraintsDto;
  deadlineSeconds?: number;
}

export interface RepositorySnapshotDto {
  id: string;
  owner: string;
  repository: string;
  baseBranch: string;
  lockSha256: string;
}

export interface ModelSnapshotDto {
  provider: string;
  model: string;
}

export interface LifecycleEventDto {
  sequence: string;
  eventTime: string;
  stage: LifecycleStageDto;
  outcome: LifecycleOutcomeDto;
  reasonCode?: string;
}

export interface ProviderTokenUsageDto {
  inputTokens?: string;
  outputTokens?: string;
  cachedInputTokens?: string;
  reasoningTokens?: string;
  totalTokens?: string;
}

export interface EstimatedModelCostDto {
  label: string;
  amountMicros: string;
  currency: string;
  pricingVersion: string;
  pricedInputTokens: string;
  pricedCachedInputTokens: string;
  pricedOutputTokens: string;
  inputRateMicrosPerMillion: string;
  cachedInputRateMicrosPerMillion: string;
  outputRateMicrosPerMillion: string;
}

export interface ModelUsageInteractionDto {
  callId: string;
  stage: ModelUsageStageDto;
  provider: string;
  model: string;
  resolvedModel?: string;
  startedAt: string;
  completedAt: string;
  latencySeconds: number;
  outcome: ModelInteractionOutcomeDto;
  tokens: ProviderTokenUsageDto;
  estimatedCost?: EstimatedModelCostDto;
}

export interface ModelUsageSummaryDto extends ProviderTokenUsageDto {
  interactionCount: number;
  unavailableInteractionCount: number;
  estimatedCostMicros?: string;
  estimatedCostCurrency?: string;
  pricingVersion?: string;
}

export interface StageModelUsageDto {
  stage: ModelUsageStageDto;
  summary: ModelUsageSummaryDto;
}

export interface TaskModelUsageDto {
  collectionStatus: ModelUsageCollectionStatusDto;
  interactions: ModelUsageInteractionDto[];
  byStage: StageModelUsageDto[];
  total: ModelUsageSummaryDto;
}

export interface TaskFailureDto {
  reason: string;
  message: string;
}

export interface DevelopmentTaskDto {
  id: string;
  input: DevelopmentTaskInputDto;
  repository: RepositorySnapshotDto;
  model: ModelSnapshotDto;
  preparationId?: string;
  retryOfTaskId?: string;
  createdAt: string;
  updatedAt: string;
  deadlineAt?: string;
  phase: DevelopmentTaskPhaseDto;
  requestedInput?: string;
  candidateCommit?: string;
  validationManifestDigest?: string;
  policyOrTestEdits?: boolean;
  scopeViolations: string[];
  draftPrUrl?: string;
  failure?: TaskFailureDto;
  lifecycleComplete: boolean;
  lifecycle: LifecycleEventDto[];
  modelUsage: TaskModelUsageDto;
  providedInput?: string;
  resourceVersion: string;
}

export interface DevelopmentTaskSummaryDto {
  id: string;
  repositoryId: string;
  repositoryOwner: string;
  repositoryName: string;
  startRevision: string;
  request: string;
  model: ModelSnapshotDto;
  retryOfTaskId?: string;
  createdAt: string;
  updatedAt: string;
  phase: DevelopmentTaskPhaseDto;
  candidateCommit?: string;
  draftPrUrl?: string;
  failure?: TaskFailureDto;
  modelUsage: ModelUsageSummaryDto;
  resourceVersion: string;
}

export interface DevelopmentTaskPageDto {
  tasks: DevelopmentTaskSummaryDto[];
  page: PageDto;
}

export interface EvidenceArtifactDto {
  name: string;
  sha256: string;
}

export interface DevelopmentTaskEvidenceDto {
  manifestDigest: string;
  outcome: EvidenceOutcomeDto;
  checkExitCode: number;
  startRevision: string;
  candidateCommit: string;
  lockSha256: string;
  changeConstraints?: ChangeConstraintsDto;
  preparationId?: string;
  preparationReused: boolean;
  policyEdited: boolean;
  testsEdited: boolean;
  scopeViolations: string[];
  changedPaths: string[];
  artifacts: EvidenceArtifactDto[];
}

export interface CreateDevelopmentTaskInput {
  idempotencyKey: string;
  input: DevelopmentTaskInputDto;
}

export interface SaveRepositoryInput {
  idempotencyKey: string;
  repository: RepositoryEnrollmentDto;
}

export interface PrepareRepositoryInput {
  idempotencyKey: string;
  repositoryId: string;
  revision: string;
}

export interface TaskActionInput {
  idempotencyKey: string;
  taskId: string;
}

export interface ProvideTaskInput extends TaskActionInput {
  input: string;
}

export interface RequestContext {
  requestId: string;
  signal?: AbortSignal;
  deadlineMs?: number;
}
