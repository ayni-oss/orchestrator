import {
  credentials,
  Metadata,
  type CallOptions,
  type ClientOptions,
  type ClientUnaryCall,
  type ServiceError,
} from "@grpc/grpc-js";
import {
  DevelopmentTaskPhase,
  DevelopmentTaskServiceClient,
  ProviderServiceClient,
  RepositoryServiceClient,
  type CancelDevelopmentTaskResponse,
  type CreateDevelopmentTaskResponse,
  type GetDevelopmentTaskEvidenceResponse,
  type GetDevelopmentTaskResponse,
  type GetPreparationResponse,
  type GetRepositoryResponse,
  type ListDevelopmentTasksResponse,
  type ListPreparationsResponse,
  type ListProviderModelsResponse,
  type ListRepositoriesResponse,
  type PrepareRepositoryResponse,
  type ProvideDevelopmentTaskInputResponse,
  type RetryDevelopmentTaskResponse,
  type SaveRepositoryResponse,
  type WatchDevelopmentTaskResponse,
} from "@ayni-orchestrator/grpc-bindings/server";
import { randomUUID } from "node:crypto";
import type {
  CreateDevelopmentTaskInput,
  DevelopmentTaskDto,
  DevelopmentTaskEvidenceDto,
  DevelopmentTaskPageDto,
  DevelopmentTaskPhaseDto,
  PageInput,
  PreparationDto,
  PreparationPageDto,
  PrepareRepositoryInput,
  ProviderModelPageDto,
  ProvideTaskInput,
  RepositoryDto,
  RepositoryPageDto,
  RequestContext,
  SaveRepositoryInput,
  TaskActionInput,
} from "./dto.js";
import { BffError, toBffError } from "./errors.js";
import {
  BffMappingError,
  mapDevelopmentTask,
  mapDevelopmentTaskSummary,
  mapEvidence,
  mapPreparation,
  mapProviderModel,
  mapRepository,
  mapTaskPhase,
  toProtoTaskInput,
} from "./mapping.js";

const MAX_REQUEST_BYTES = 1024 * 1024;
const MAX_RESPONSE_BYTES = 4 * 1024 * 1024;
const DEFAULT_UNARY_DEADLINE_MS = 10_000;
const MAX_UNARY_DEADLINE_MS = 30_000;
const DEFAULT_STREAM_DEADLINE_MS = 5 * 60_000;
const REQUEST_ID = /^[A-Za-z0-9._-]{1,64}$/u;
const LOOPBACK_ADDRESS = /^(?:127\.0\.0\.1|localhost):(?:[1-9][0-9]{0,4})$|^\[::1\]:(?:[1-9][0-9]{0,4})$/u;
const RESOURCE_VERSION = /^(?:0|[1-9][0-9]{0,19})$/u;
const MAX_UINT64 = 18_446_744_073_709_551_615n;

interface BffClientConfiguration {
  address?: string;
}

type UnaryInvoker<T> = (
  metadata: Metadata,
  options: Partial<CallOptions>,
  callback: (error: ServiceError | null, response: T) => void,
) => ClientUnaryCall;

function required<T>(value: T | null | undefined): T {
  if (value === undefined || value === null) throw new BffMappingError();
  return value;
}

function requestId(value: string): string {
  return REQUEST_ID.test(value) ? value : randomUUID();
}

function page(value: PageInput = {}): { pageSize: number; pageToken: string } {
  const pageSize = value.pageSize ?? 0;
  const pageToken = value.pageToken ?? "";
  if (!Number.isInteger(pageSize) || pageSize < 0 || pageSize > 100 || Buffer.byteLength(pageToken) > 512) {
    throw new BffError({
      status: 400,
      reason: "invalid_input",
      message: "the page selection is invalid",
      retryable: false,
    });
  }
  return { pageSize, pageToken };
}

function resourceVersion(value: string | undefined): string | undefined {
  if (value !== undefined && (!RESOURCE_VERSION.test(value) || BigInt(value) > MAX_UINT64)) {
    throw new BffError({
      status: 400,
      reason: "invalid_input",
      message: "the task update cursor is invalid",
      retryable: false,
    });
  }
  return value;
}

function phase(value: DevelopmentTaskPhaseDto | undefined): DevelopmentTaskPhase | undefined {
  switch (value) {
    case undefined:
      return undefined;
    case "queued":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_QUEUED;
    case "coding":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_CODING;
    case "waiting_for_input":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_WAITING_FOR_INPUT;
    case "validation_queued":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_VALIDATION_QUEUED;
    case "validating":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_VALIDATING;
    case "pr_queued":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_PR_QUEUED;
    case "preparing_pr":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_PREPARING_PR;
    case "completed":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_COMPLETED;
    case "failed":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_FAILED;
    case "cancelled":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_CANCELLED;
    case "unsupported":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_UNSUPPORTED;
    case "unknown":
      return DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_UNSPECIFIED;
  }
}

export class OrchestratorBff {
  readonly #repositories: RepositoryServiceClient;
  readonly #providers: ProviderServiceClient;
  readonly #tasks: DevelopmentTaskServiceClient;

  constructor(configuration: BffClientConfiguration = {}) {
    const address = configuration.address ?? "127.0.0.1:50051";
    if (!LOOPBACK_ADDRESS.test(address)) {
      throw new Error("the supported BFF backend address must use loopback");
    }
    const options: Partial<ClientOptions> = {
      "grpc.max_receive_message_length": MAX_RESPONSE_BYTES,
      "grpc.max_send_message_length": MAX_REQUEST_BYTES,
    };
    const channelCredentials = credentials.createInsecure();
    this.#repositories = new RepositoryServiceClient(address, channelCredentials, options);
    this.#providers = new ProviderServiceClient(address, channelCredentials, options);
    this.#tasks = new DevelopmentTaskServiceClient(address, channelCredentials, options);
  }

  close(): void {
    this.#repositories.close();
    this.#providers.close();
    this.#tasks.close();
  }

  async listRepositories(selection: PageInput, context: RequestContext): Promise<RepositoryPageDto> {
    const response = await this.#unary<ListRepositoriesResponse>(context, (metadata, options, callback) =>
      this.#repositories.listRepositories({ page: page(selection) }, metadata, options, callback));
    return {
      repositories: response.repositories.map(mapRepository),
      page: { nextPageToken: required(response.page).nextPageToken },
    };
  }

  async getRepository(repositoryId: string, context: RequestContext): Promise<RepositoryDto> {
    const response = await this.#unary<GetRepositoryResponse>(context, (metadata, options, callback) =>
      this.#repositories.getRepository({ repositoryId }, metadata, options, callback));
    return mapRepository(required(response.repository));
  }

  async saveRepository(input: SaveRepositoryInput, context: RequestContext): Promise<RepositoryDto> {
    const response = await this.#unary<SaveRepositoryResponse>(context, (metadata, options, callback) =>
      this.#repositories.saveRepository({
        idempotencyKey: input.idempotencyKey,
        repository: {
          id: input.repository.id,
          owner: input.repository.owner,
          repository: input.repository.repository,
          installationId: input.repository.installationId,
          baseBranch: input.repository.baseBranch,
          lockSha256: input.repository.lockSha256,
        },
      }, metadata, options, callback));
    return mapRepository(required(response.repository));
  }

  async listPreparations(repositoryId: string, selection: PageInput, context: RequestContext): Promise<PreparationPageDto> {
    const response = await this.#unary<ListPreparationsResponse>(context, (metadata, options, callback) =>
      this.#repositories.listPreparations({ repositoryId, page: page(selection) }, metadata, options, callback));
    return {
      preparations: response.preparations.map(mapPreparation),
      page: { nextPageToken: required(response.page).nextPageToken },
    };
  }

  async getPreparation(preparationId: string, context: RequestContext): Promise<PreparationDto> {
    const response = await this.#unary<GetPreparationResponse>(context, (metadata, options, callback) =>
      this.#repositories.getPreparation({ preparationId }, metadata, options, callback));
    return mapPreparation(required(response.preparation));
  }

  async prepareRepository(input: PrepareRepositoryInput, context: RequestContext): Promise<{ preparation: PreparationDto; reused: boolean }> {
    const response = await this.#unary<PrepareRepositoryResponse>(context, (metadata, options, callback) =>
      this.#repositories.prepareRepository({
        idempotencyKey: input.idempotencyKey,
        repositoryId: input.repositoryId,
        revision: input.revision,
      }, metadata, options, callback));
    return { preparation: mapPreparation(required(response.preparation)), reused: response.reused };
  }

  async listProviderModels(selection: PageInput, context: RequestContext): Promise<ProviderModelPageDto> {
    const response = await this.#unary<ListProviderModelsResponse>(context, (metadata, options, callback) =>
      this.#providers.listProviderModels({ page: page(selection) }, metadata, options, callback));
    const observedAt = required(response.observedAt);
    if (!Number.isFinite(observedAt.getTime())) throw new BffMappingError();
    return {
      provider: response.provider,
      models: response.models.map(mapProviderModel),
      observedAt: observedAt.toISOString(),
      page: { nextPageToken: required(response.page).nextPageToken },
    };
  }

  async listDevelopmentTasks(
    selection: PageInput,
    phaseFilter: DevelopmentTaskPhaseDto | undefined,
    context: RequestContext,
  ): Promise<DevelopmentTaskPageDto> {
    const response = await this.#unary<ListDevelopmentTasksResponse>(context, (metadata, options, callback) =>
      this.#tasks.listDevelopmentTasks({
        page: page(selection),
        ...(phase(phaseFilter) === undefined ? {} : { phase: phase(phaseFilter) }),
      }, metadata, options, callback));
    return {
      tasks: response.tasks.map(mapDevelopmentTaskSummary),
      page: { nextPageToken: required(response.page).nextPageToken },
    };
  }

  async getDevelopmentTask(taskId: string, context: RequestContext): Promise<DevelopmentTaskDto> {
    const response = await this.#unary<GetDevelopmentTaskResponse>(context, (metadata, options, callback) =>
      this.#tasks.getDevelopmentTask({ taskId }, metadata, options, callback));
    return mapDevelopmentTask(required(response.task));
  }

  async createDevelopmentTask(input: CreateDevelopmentTaskInput, context: RequestContext): Promise<DevelopmentTaskDto> {
    const response = await this.#unary<CreateDevelopmentTaskResponse>(context, (metadata, options, callback) =>
      this.#tasks.createDevelopmentTask({
        idempotencyKey: input.idempotencyKey,
        input: toProtoTaskInput(input.input),
      }, metadata, options, callback));
    return mapDevelopmentTask(required(response.task));
  }

  async cancelDevelopmentTask(input: TaskActionInput, context: RequestContext): Promise<DevelopmentTaskDto> {
    const response = await this.#unary<CancelDevelopmentTaskResponse>(context, (metadata, options, callback) =>
      this.#tasks.cancelDevelopmentTask(input, metadata, options, callback));
    return mapDevelopmentTask(required(response.task));
  }

  async retryDevelopmentTask(input: TaskActionInput, context: RequestContext): Promise<DevelopmentTaskDto> {
    const response = await this.#unary<RetryDevelopmentTaskResponse>(context, (metadata, options, callback) =>
      this.#tasks.retryDevelopmentTask(input, metadata, options, callback));
    return mapDevelopmentTask(required(response.task));
  }

  async provideDevelopmentTaskInput(input: ProvideTaskInput, context: RequestContext): Promise<DevelopmentTaskDto> {
    const response = await this.#unary<ProvideDevelopmentTaskInputResponse>(context, (metadata, options, callback) =>
      this.#tasks.provideDevelopmentTaskInput(input, metadata, options, callback));
    return mapDevelopmentTask(required(response.task));
  }

  async getDevelopmentTaskEvidence(taskId: string, context: RequestContext): Promise<DevelopmentTaskEvidenceDto> {
    const response = await this.#unary<GetDevelopmentTaskEvidenceResponse>(context, (metadata, options, callback) =>
      this.#tasks.getDevelopmentTaskEvidence({ taskId }, metadata, options, callback));
    return mapEvidence(required(response.evidence));
  }

  watchDevelopmentTask(
    taskId: string,
    afterResourceVersion: string | undefined,
    context: RequestContext,
    onUpdate: (task: DevelopmentTaskDto) => void | Promise<void>,
  ): Promise<void> {
    const acceptedRequestId = requestId(context.requestId);
    const metadata = new Metadata();
    metadata.set("x-request-id", acceptedRequestId);
    const duration = Math.min(Math.max(context.deadlineMs ?? DEFAULT_STREAM_DEADLINE_MS, 100), DEFAULT_STREAM_DEADLINE_MS);
    const cursor = resourceVersion(afterResourceVersion);
    const stream = this.#tasks.watchDevelopmentTask({
      taskId,
      ...(cursor === undefined ? {} : { afterResourceVersion: cursor }),
    }, metadata, { deadline: Date.now() + duration });

    return new Promise((resolve, reject) => {
      let settled = false;
      const finish = (error?: unknown): void => {
        if (settled) return;
        settled = true;
        context.signal?.removeEventListener("abort", abort);
        if (error === undefined || context.signal?.aborted === true) resolve();
        else reject(toBffError(error, acceptedRequestId));
      };
      const abort = (): void => {
        stream.cancel();
        finish();
      };
      if (context.signal?.aborted === true) {
        abort();
        return;
      }
      context.signal?.addEventListener("abort", abort, { once: true });
      stream.on("data", (response: WatchDevelopmentTaskResponse) => {
        stream.pause();
        Promise.resolve()
          .then(() => onUpdate(mapDevelopmentTask(required(response.task))))
          .then(() => stream.resume())
          .catch((error: unknown) => {
            stream.cancel();
            finish(error);
          });
      });
      stream.on("error", finish);
      stream.on("end", () => finish());
    });
  }

  #unary<T>(context: RequestContext, invoke: UnaryInvoker<T>): Promise<T> {
    const acceptedRequestId = requestId(context.requestId);
    const metadata = new Metadata();
    metadata.set("x-request-id", acceptedRequestId);
    const duration = Math.min(Math.max(context.deadlineMs ?? DEFAULT_UNARY_DEADLINE_MS, 100), MAX_UNARY_DEADLINE_MS);
    if (context.signal?.aborted === true) {
      return Promise.reject(new BffError({
        status: 499,
        reason: "cancelled",
        message: "the request was cancelled",
        requestId: acceptedRequestId,
        retryable: false,
      }));
    }
    return new Promise<T>((resolve, reject) => {
      let call: ClientUnaryCall | undefined;
      let settled = false;
      const finish = (error: unknown, response?: T): void => {
        if (settled) return;
        settled = true;
        context.signal?.removeEventListener("abort", abort);
        if (error !== null && error !== undefined) reject(toBffError(error, acceptedRequestId));
        else if (response === undefined) reject(toBffError(new BffMappingError(), acceptedRequestId));
        else resolve(response);
      };
      const abort = (): void => call?.cancel();
      context.signal?.addEventListener("abort", abort, { once: true });
      call = invoke(metadata, { deadline: Date.now() + duration }, (error, response) => finish(error, response));
      if (context.signal?.aborted === true) call.cancel();
    });
  }
}

export function phaseFromWire(value: DevelopmentTaskPhase): DevelopmentTaskPhaseDto {
  return mapTaskPhase(value);
}
