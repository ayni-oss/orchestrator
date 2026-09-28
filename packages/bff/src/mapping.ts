import {
  DevelopmentTaskPhase,
  ErrorReason,
  EvidenceOutcome,
  LifecycleOutcome,
  LifecycleStage,
  ModelInteractionOutcome,
  ModelUsageCollectionStatus,
  ModelUsageStage,
  PreparationStatus,
  type ChangeConstraints,
  type DevelopmentTask as ProtoDevelopmentTask,
  type DevelopmentTaskEvidence as ProtoDevelopmentTaskEvidence,
  type DevelopmentTaskInput as ProtoDevelopmentTaskInput,
  type DevelopmentTaskSummary as ProtoDevelopmentTaskSummary,
  type EstimatedModelCost,
  type LifecycleEvent,
  type ModelUsageInteraction,
  type ModelUsageSummary,
  type ProviderModel,
  type ProviderTokenUsage,
  type RepositoryBinding,
  type RepositoryEnrollment,
  type StageModelUsage,
  type TaskFailure,
  type TaskModelUsage,
} from "@ayni-orchestrator/grpc-bindings/server";
import type {
  ChangeConstraintsDto,
  DevelopmentTaskDto,
  DevelopmentTaskEvidenceDto,
  DevelopmentTaskInputDto,
  DevelopmentTaskPhaseDto,
  DevelopmentTaskSummaryDto,
  EstimatedModelCostDto,
  EvidenceOutcomeDto,
  LifecycleEventDto,
  LifecycleOutcomeDto,
  LifecycleStageDto,
  ModelInteractionOutcomeDto,
  ModelSnapshotDto,
  ModelUsageCollectionStatusDto,
  ModelUsageInteractionDto,
  ModelUsageStageDto,
  ModelUsageSummaryDto,
  PreparationDto,
  PreparationStatusDto,
  ProviderModelDto,
  ProviderTokenUsageDto,
  RepositoryDto,
  RepositoryEnrollmentDto,
  StageModelUsageDto,
  TaskFailureDto,
  TaskModelUsageDto,
} from "./dto.js";

interface DurationValue {
  seconds: string;
  nanos: number;
}

export class BffMappingError extends Error {
  constructor(message = "the control plane returned an incomplete response") {
    super(message);
    this.name = "BffMappingError";
  }
}

function required<T>(value: T | null | undefined): T {
  if (value === undefined || value === null) {
    throw new BffMappingError();
  }
  return value;
}

function date(value: Date | undefined): string | undefined {
  if (value === undefined || !Number.isFinite(value.getTime())) return undefined;
  return value.toISOString();
}

function durationSeconds(value: DurationValue | undefined): number | undefined {
  if (value === undefined) return undefined;
  const seconds = Number(value.seconds);
  const result = seconds + value.nanos / 1_000_000_000;
  return Number.isFinite(result) && result >= 0 ? result : undefined;
}

function requiredDate(value: Date | undefined): string {
  return required(date(value));
}

function requiredDurationSeconds(value: DurationValue | undefined): number {
  return required(durationSeconds(value));
}

function optional<T extends object>(key: string, value: unknown): T | object {
  return value === undefined ? {} : { [key]: value };
}

export function mapChangeConstraints(value: ChangeConstraints): ChangeConstraintsDto {
  return {
    protectedPaths: value.protectedPaths.map(String),
    allowTestEdits: value.allowTestEdits,
  };
}

export function mapPreparationStatus(value: PreparationStatus): PreparationStatusDto {
  switch (value) {
    case PreparationStatus.PREPARATION_STATUS_QUEUED:
      return "queued";
    case PreparationStatus.PREPARATION_STATUS_PREPARING:
      return "preparing";
    case PreparationStatus.PREPARATION_STATUS_READY:
      return "ready";
    case PreparationStatus.PREPARATION_STATUS_SETUP_REQUIRED:
      return "setup_required";
    case PreparationStatus.PREPARATION_STATUS_FAILED:
      return "failed";
    default:
      return "unknown";
  }
}

export function mapPreparation(value: import("@ayni-orchestrator/grpc-bindings/server").Preparation): PreparationDto {
  const diagnostic = value.diagnostic;
  const duration = durationSeconds(value.duration);
  return {
    id: value.id,
    repositoryId: value.repositoryId,
    revision: value.revision,
    lockSha256: value.lockSha256,
    status: mapPreparationStatus(value.status),
    reuseCount: value.reuseCount,
    ...optional("diagnostic", diagnostic),
    ...optional("durationSeconds", duration),
  } as PreparationDto;
}

export function mapRepositoryEnrollment(value: RepositoryEnrollment): RepositoryEnrollmentDto {
  return {
    id: value.id,
    owner: value.owner,
    repository: value.repository,
    installationId: value.installationId,
    baseBranch: value.baseBranch,
    lockSha256: value.lockSha256,
  };
}

export function mapRepository(value: RepositoryBinding): RepositoryDto {
  const latestPreparation = value.latestPreparation === undefined
    ? undefined
    : mapPreparation(value.latestPreparation);
  return {
    enrollment: mapRepositoryEnrollment(required(value.enrollment)),
    ...optional("latestPreparation", latestPreparation),
  } as RepositoryDto;
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

export function mapTaskPhase(value: DevelopmentTaskPhase): DevelopmentTaskPhaseDto {
  switch (value) {
    case DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_QUEUED:
      return "queued";
    case DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_CODING:
      return "coding";
    case DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_WAITING_FOR_INPUT:
      return "waiting_for_input";
    case DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_VALIDATION_QUEUED:
      return "validation_queued";
    case DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_VALIDATING:
      return "validating";
    case DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_PR_QUEUED:
      return "pr_queued";
    case DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_PREPARING_PR:
      return "preparing_pr";
    case DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_COMPLETED:
      return "completed";
    case DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_FAILED:
      return "failed";
    case DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_CANCELLED:
      return "cancelled";
    case DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_UNSUPPORTED:
      return "unsupported";
    default:
      return "unknown";
  }
}

function mapLifecycleStage(value: LifecycleStage): LifecycleStageDto {
  switch (value) {
    case LifecycleStage.LIFECYCLE_STAGE_QUEUE:
      return "queue";
    case LifecycleStage.LIFECYCLE_STAGE_CODING:
      return "coding";
    case LifecycleStage.LIFECYCLE_STAGE_VALIDATION:
      return "validation";
    case LifecycleStage.LIFECYCLE_STAGE_PR_PREPARATION:
      return "pr_preparation";
    default:
      return "unknown";
  }
}

function mapLifecycleOutcome(value: LifecycleOutcome): LifecycleOutcomeDto {
  switch (value) {
    case LifecycleOutcome.LIFECYCLE_OUTCOME_QUEUED:
      return "queued";
    case LifecycleOutcome.LIFECYCLE_OUTCOME_STARTED:
      return "started";
    case LifecycleOutcome.LIFECYCLE_OUTCOME_SUCCEEDED:
      return "succeeded";
    case LifecycleOutcome.LIFECYCLE_OUTCOME_BLOCKED:
      return "blocked";
    case LifecycleOutcome.LIFECYCLE_OUTCOME_FAILED:
      return "failed";
    case LifecycleOutcome.LIFECYCLE_OUTCOME_CANCELLED:
      return "cancelled";
    case LifecycleOutcome.LIFECYCLE_OUTCOME_UNSUPPORTED:
      return "unsupported";
    default:
      return "unknown";
  }
}

function mapUsageStage(value: ModelUsageStage): ModelUsageStageDto {
  switch (value) {
    case ModelUsageStage.MODEL_USAGE_STAGE_CODING:
      return "coding";
    case ModelUsageStage.MODEL_USAGE_STAGE_PR_PREPARATION:
      return "pr_preparation";
    default:
      return "unknown";
  }
}

function mapInteractionOutcome(value: ModelInteractionOutcome): ModelInteractionOutcomeDto {
  switch (value) {
    case ModelInteractionOutcome.MODEL_INTERACTION_OUTCOME_SUCCEEDED:
      return "succeeded";
    case ModelInteractionOutcome.MODEL_INTERACTION_OUTCOME_FAILED:
      return "failed";
    case ModelInteractionOutcome.MODEL_INTERACTION_OUTCOME_CANCELLED:
      return "cancelled";
    default:
      return "unknown";
  }
}

function mapCollectionStatus(value: ModelUsageCollectionStatus): ModelUsageCollectionStatusDto {
  switch (value) {
    case ModelUsageCollectionStatus.MODEL_USAGE_COLLECTION_STATUS_UNAVAILABLE:
      return "unavailable";
    case ModelUsageCollectionStatus.MODEL_USAGE_COLLECTION_STATUS_COLLECTING:
      return "collecting";
    case ModelUsageCollectionStatus.MODEL_USAGE_COLLECTION_STATUS_COMPLETE:
      return "complete";
    default:
      return "unknown";
  }
}

function mapEvidenceOutcome(value: EvidenceOutcome): EvidenceOutcomeDto {
  switch (value) {
    case EvidenceOutcome.EVIDENCE_OUTCOME_SUCCESS:
      return "success";
    case EvidenceOutcome.EVIDENCE_OUTCOME_QUALITY_FAILURE:
      return "quality_failure";
    case EvidenceOutcome.EVIDENCE_OUTCOME_POLICY_REJECTED:
      return "policy_rejected";
    case EvidenceOutcome.EVIDENCE_OUTCOME_DEPENDENCY_CHANGE_UNSUPPORTED:
      return "dependency_change_unsupported";
    case EvidenceOutcome.EVIDENCE_OUTCOME_SCOPE_REJECTED:
      return "scope_rejected";
    default:
      return "unknown";
  }
}

function mapErrorReason(value: ErrorReason): string {
  switch (value) {
    case ErrorReason.ERROR_REASON_INVALID_IDENTIFIER:
      return "invalid_identifier";
    case ErrorReason.ERROR_REASON_INVALID_INPUT:
      return "invalid_input";
    case ErrorReason.ERROR_REASON_IDEMPOTENCY_KEY_REUSED:
      return "idempotency_key_reused";
    case ErrorReason.ERROR_REASON_RESOURCE_NOT_FOUND:
      return "resource_not_found";
    case ErrorReason.ERROR_REASON_CONFLICT:
      return "conflict";
    case ErrorReason.ERROR_REASON_FAILED_PRECONDITION:
      return "failed_precondition";
    case ErrorReason.ERROR_REASON_TASK_NOT_ACTIONABLE:
      return "task_not_actionable";
    case ErrorReason.ERROR_REASON_PROVIDER_UNAVAILABLE:
      return "provider_unavailable";
    case ErrorReason.ERROR_REASON_MODEL_UNAVAILABLE:
      return "model_unavailable";
    case ErrorReason.ERROR_REASON_REPOSITORY_ACCESS:
      return "repository_access";
    case ErrorReason.ERROR_REASON_EVIDENCE_UNAVAILABLE:
      return "evidence_unavailable";
    case ErrorReason.ERROR_REASON_CANCELLED:
      return "cancelled";
    case ErrorReason.ERROR_REASON_DEADLINE_EXCEEDED:
      return "deadline_exceeded";
    case ErrorReason.ERROR_REASON_INTERNAL:
      return "internal";
    case ErrorReason.ERROR_REASON_INVALID_SOURCE_REVISION:
      return "invalid_source_revision";
    case ErrorReason.ERROR_REASON_INVALID_LOCK_DIGEST:
      return "invalid_lock_digest";
    case ErrorReason.ERROR_REASON_REPOSITORY_NOT_READY:
      return "repository_not_ready";
    case ErrorReason.ERROR_REASON_TASK_PHASE_NOT_ELIGIBLE:
      return "task_phase_not_eligible";
    case ErrorReason.ERROR_REASON_TASK_DEADLINE_EXPIRED:
      return "task_deadline_expired";
    default:
      return "unspecified";
  }
}

function mapTaskInput(value: ProtoDevelopmentTaskInput): DevelopmentTaskInputDto {
  return {
    repositoryId: value.repositoryId,
    model: value.model,
    startRevision: value.startRevision,
    request: value.request,
    acceptanceCriteria: value.acceptanceCriteria.map(String),
    changeConstraints: mapChangeConstraints(required(value.changeConstraints)),
    ...optional("deadlineSeconds", durationSeconds(value.deadline)),
  } as DevelopmentTaskInputDto;
}

function mapModelSnapshot(value: import("@ayni-orchestrator/grpc-bindings/server").ModelSnapshot): ModelSnapshotDto {
  return { provider: value.provider, model: value.model };
}

function mapLifecycleEvent(value: LifecycleEvent): LifecycleEventDto {
  return {
    sequence: value.sequence,
    eventTime: requiredDate(value.eventTime),
    stage: mapLifecycleStage(value.stage),
    outcome: mapLifecycleOutcome(value.outcome),
    ...optional("reasonCode", value.reasonCode),
  } as LifecycleEventDto;
}

function mapTokens(value: ProviderTokenUsage): ProviderTokenUsageDto {
  return {
    ...optional("inputTokens", value.inputTokens),
    ...optional("outputTokens", value.outputTokens),
    ...optional("cachedInputTokens", value.cachedInputTokens),
    ...optional("reasoningTokens", value.reasoningTokens),
    ...optional("totalTokens", value.totalTokens),
  } as ProviderTokenUsageDto;
}

function mapEstimatedCost(value: EstimatedModelCost): EstimatedModelCostDto {
  return {
    label: value.label,
    amountMicros: value.amountMicros,
    currency: value.currency,
    pricingVersion: value.pricingVersion,
    pricedInputTokens: value.pricedInputTokens,
    pricedCachedInputTokens: value.pricedCachedInputTokens,
    pricedOutputTokens: value.pricedOutputTokens,
    inputRateMicrosPerMillion: value.inputRateMicrosPerMillion,
    cachedInputRateMicrosPerMillion: value.cachedInputRateMicrosPerMillion,
    outputRateMicrosPerMillion: value.outputRateMicrosPerMillion,
  };
}

function mapInteraction(value: ModelUsageInteraction): ModelUsageInteractionDto {
  return {
    callId: value.callId,
    stage: mapUsageStage(value.stage),
    provider: value.provider,
    model: value.model,
    startedAt: requiredDate(value.startedAt),
    completedAt: requiredDate(value.completedAt),
    latencySeconds: requiredDurationSeconds(value.latency),
    outcome: mapInteractionOutcome(value.outcome),
    tokens: mapTokens(required(value.tokens)),
    ...optional("resolvedModel", value.resolvedModel),
    ...optional("estimatedCost", value.estimatedCost === undefined ? undefined : mapEstimatedCost(value.estimatedCost)),
  } as ModelUsageInteractionDto;
}

export function mapUsageSummary(value: ModelUsageSummary): ModelUsageSummaryDto {
  return {
    interactionCount: value.interactionCount,
    unavailableInteractionCount: value.unavailableInteractionCount,
    ...mapTokens(value),
    ...optional("estimatedCostMicros", value.estimatedCostMicros),
    ...optional("estimatedCostCurrency", value.estimatedCostCurrency),
    ...optional("pricingVersion", value.pricingVersion),
  } as ModelUsageSummaryDto;
}

function mapStageUsage(value: StageModelUsage): StageModelUsageDto {
  return { stage: mapUsageStage(value.stage), summary: mapUsageSummary(required(value.summary)) };
}

function mapTaskUsage(value: TaskModelUsage): TaskModelUsageDto {
  return {
    collectionStatus: mapCollectionStatus(value.collectionStatus),
    interactions: value.interactions.map(mapInteraction),
    byStage: value.byStage.map(mapStageUsage),
    total: mapUsageSummary(required(value.total)),
  };
}

function mapFailure(value: TaskFailure | undefined): TaskFailureDto | undefined {
  return value === undefined ? undefined : { reason: mapErrorReason(value.reason), message: value.message };
}

export function mapDevelopmentTask(value: ProtoDevelopmentTask): DevelopmentTaskDto {
  const repository = required(value.repository);
  return {
    id: value.id,
    input: mapTaskInput(required(value.input)),
    repository: {
      id: repository.id,
      owner: repository.owner,
      repository: repository.repository,
      baseBranch: repository.baseBranch,
      lockSha256: repository.lockSha256,
    },
    model: mapModelSnapshot(required(value.model)),
    createdAt: requiredDate(value.createdAt),
    updatedAt: requiredDate(value.updatedAt),
    phase: mapTaskPhase(value.phase),
    scopeViolations: value.scopeViolations.map(String),
    lifecycleComplete: value.lifecycleComplete,
    lifecycle: value.lifecycle.map(mapLifecycleEvent),
    modelUsage: mapTaskUsage(required(value.modelUsage)),
    resourceVersion: value.resourceVersion,
    ...optional("preparationId", value.preparationId),
    ...optional("retryOfTaskId", value.retryOfTaskId),
    ...optional("deadlineAt", date(value.deadlineAt)),
    ...optional("requestedInput", value.requestedInput),
    ...optional("candidateCommit", value.candidateCommit),
    ...optional("validationManifestDigest", value.validationManifestDigest),
    ...optional("policyOrTestEdits", value.policyOrTestEdits),
    ...optional("draftPrUrl", value.draftPrUrl),
    ...optional("failure", mapFailure(value.failure)),
    ...optional("providedInput", value.providedInput),
  } as DevelopmentTaskDto;
}

export function mapDevelopmentTaskSummary(value: ProtoDevelopmentTaskSummary): DevelopmentTaskSummaryDto {
  return {
    id: value.id,
    repositoryId: value.repositoryId,
    repositoryOwner: value.repositoryOwner,
    repositoryName: value.repositoryName,
    startRevision: value.startRevision,
    request: value.request,
    model: mapModelSnapshot(required(value.model)),
    createdAt: requiredDate(value.createdAt),
    updatedAt: requiredDate(value.updatedAt),
    phase: mapTaskPhase(value.phase),
    modelUsage: mapUsageSummary(required(value.modelUsage)),
    resourceVersion: value.resourceVersion,
    ...optional("retryOfTaskId", value.retryOfTaskId),
    ...optional("candidateCommit", value.candidateCommit),
    ...optional("draftPrUrl", value.draftPrUrl),
    ...optional("failure", mapFailure(value.failure)),
  } as DevelopmentTaskSummaryDto;
}

export function mapEvidence(value: ProtoDevelopmentTaskEvidence): DevelopmentTaskEvidenceDto {
  return {
    manifestDigest: value.manifestDigest,
    outcome: mapEvidenceOutcome(value.outcome),
    checkExitCode: value.checkExitCode,
    startRevision: value.startRevision,
    candidateCommit: value.candidateCommit,
    lockSha256: value.lockSha256,
    preparationReused: value.preparationReused,
    policyEdited: value.policyEdited,
    testsEdited: value.testsEdited,
    scopeViolations: value.scopeViolations.map(String),
    changedPaths: value.changedPaths.map(String),
    artifacts: value.artifacts.map((artifact) => ({ name: artifact.name, sha256: artifact.sha256 })),
    ...optional("changeConstraints", value.changeConstraints === undefined ? undefined : mapChangeConstraints(value.changeConstraints)),
    ...optional("preparationId", value.preparationId),
  } as DevelopmentTaskEvidenceDto;
}

export function toProtoTaskInput(value: DevelopmentTaskInputDto): ProtoDevelopmentTaskInput {
  return {
    repositoryId: value.repositoryId,
    model: value.model,
    startRevision: value.startRevision,
    request: value.request,
    acceptanceCriteria: value.acceptanceCriteria.map(String),
    changeConstraints: {
      protectedPaths: value.changeConstraints.protectedPaths.map(String),
      allowTestEdits: value.changeConstraints.allowTestEdits,
    },
    ...(value.deadlineSeconds === undefined
      ? {}
      : { deadline: { seconds: Math.trunc(value.deadlineSeconds).toString(), nanos: 0 } }),
  };
}
