import {
  credentials,
  Metadata,
  type CallOptions,
  type ClientOptions,
  type ClientUnaryCall,
  type ServiceError,
} from "@grpc/grpc-js";
import {
  DevelopmentTaskServiceClient,
  ReviewServiceClient,
  type ListChangeRequestsResponse,
  type ListAssessmentsResponse,
  type ListEvidenceReferencesResponse,
  ProviderServiceClient,
  RepositoryServiceClient,
  RuntimeProfileServiceClient,
  type CancelDevelopmentTaskResponse,
  type CreateDevelopmentTaskResponse,
  type CreateRepositoryConnectionResponse,
  type CreateRuntimeProfileResponse,
  type GetDevelopmentTaskResponse,
  type GetRepositoryConnectionResponse,
  type GetRuntimeProfileResponse,
  type ListDevelopmentTasksResponse,
  type ListProviderModelsResponse,
  type ListRepositoryConnectionsResponse,
  type ListRuntimeArtifactsResponse,
  type ListRuntimeProfilesResponse,
  type RetryDevelopmentTaskResponse,
  type WatchDevelopmentTaskResponse,
} from "@ayni-orchestrator/grpc-bindings/server";
import { mapReview, mapAssessment, mapEvidence } from "./reviews.js";
import { randomUUID } from "node:crypto";
import type {
  CreateDevelopmentTaskInput,
  CreateRepositoryConnectionInput,
  CreateRuntimeProfileInput,
  DevelopmentAttemptDto,
  DevelopmentTaskDto,
  DevelopmentTaskPageDto,
  DevelopmentTaskUpdateDto,
  PageInput,
  ProviderModelPageDto,
  RepositoryConnectionDto,
  RepositoryConnectionPageDto,
  RequestContext,
  RuntimeArtifactPageDto,
  RuntimeProfileDto,
  RuntimeProfilePageDto,
  TaskActionInput,
} from "./dto.js";
import { BffError, toBffError } from "./errors.js";
import {
  BffMappingError,
  mapDevelopmentAttempt,
  mapDevelopmentTask,
  mapProviderModel,
  mapRepositoryConnection,
  mapRuntimeArtifact,
  mapRuntimeProfile,
  toProtoProfileConfiguration,
  toProtoRepositoryProvider,
  toProtoTaskRequest,
} from "./mapping.js";

const MAX_REQUEST_BYTES = 1024 * 1024;
const MAX_RESPONSE_BYTES = 4 * 1024 * 1024;
const DEFAULT_UNARY_DEADLINE_MS = 10_000;
const MAX_UNARY_DEADLINE_MS = 30_000;
const DEFAULT_STREAM_DEADLINE_MS = 5 * 60_000;
const REQUEST_ID = /^[A-Za-z0-9._-]{1,64}$/u;
const LOOPBACK_ADDRESS = /^(?:127\.0\.0\.1|localhost):(?:[1-9][0-9]{0,4})$|^\[::1\]:(?:[1-9][0-9]{0,4})$/u;
const CURSOR = /^(?:0|[1-9][0-9]{0,19})$/u;
const MAX_UINT64 = 18_446_744_073_709_551_615n;

interface BffClientConfiguration { address?: string }

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

function cursor(value: string | undefined): string | undefined {
  if (value !== undefined && (!CURSOR.test(value) || BigInt(value) > MAX_UINT64)) {
    throw new BffError({
      status: 400,
      reason: "invalid_input",
      message: "the update cursor is invalid",
      retryable: false,
    });
  }
  return value;
}

function pageDto(value: { nextPageToken: string; snapshotCursor: string } | undefined): {
  nextPageToken: string;
  snapshotCursor: string;
} {
  const page = required(value);
  return { nextPageToken: page.nextPageToken, snapshotCursor: page.snapshotCursor };
}

export class OrchestratorBff {
  readonly #repositories: RepositoryServiceClient;
  readonly #profiles: RuntimeProfileServiceClient;
  readonly #providers: ProviderServiceClient;
  readonly #tasks: DevelopmentTaskServiceClient;
  readonly #reviews: ReviewServiceClient;

  constructor(configuration: BffClientConfiguration = {}) {
    const address = configuration.address ?? "127.0.0.1:50051";
    if (!LOOPBACK_ADDRESS.test(address)) throw new Error("the supported BFF backend address must use loopback");
    const options: Partial<ClientOptions> = {
      "grpc.max_receive_message_length": MAX_RESPONSE_BYTES,
      "grpc.max_send_message_length": MAX_REQUEST_BYTES,
    };
    const channelCredentials = credentials.createInsecure();
    this.#repositories = new RepositoryServiceClient(address, channelCredentials, options);
    this.#profiles = new RuntimeProfileServiceClient(address, channelCredentials, options);
    this.#providers = new ProviderServiceClient(address, channelCredentials, options);
    this.#tasks = new DevelopmentTaskServiceClient(address, channelCredentials, options);
    this.#reviews = new ReviewServiceClient(address, channelCredentials, options);
  }

  close(): void {
    this.#repositories.close();
    this.#profiles.close();
    this.#providers.close();
    this.#tasks.close();
    this.#reviews.close();
  }

  async listReviews(selection: PageInput, context: RequestContext) {
    const response = await this.#unary<ListChangeRequestsResponse>(context, (metadata, options, callback) =>
      this.#reviews.listChangeRequests({ page: page(selection) }, metadata, options, callback));
    return { reviews: response.changeRequests.map(mapReview), page: pageDto(response.page) };
  }

  async listAssessments(changeRequestId: string, selection: PageInput, context: RequestContext) {
    const response = await this.#unary<ListAssessmentsResponse>(context, (metadata, options, callback) =>
      this.#reviews.listAssessments({ changeRequestId, page: page(selection) }, metadata, options, callback));
    return { assessments: response.assessments.map(mapAssessment), page: pageDto(response.page) };
  }

  async listEvidence(assessmentId: string, selection: PageInput, context: RequestContext) {
    const response = await this.#unary<ListEvidenceReferencesResponse>(context, (metadata, options, callback) =>
      this.#reviews.listEvidenceReferences({ assessmentId, page: page(selection) }, metadata, options, callback));
    return { evidence: response.evidenceReferences.map(mapEvidence), page: pageDto(response.page) };
  }

  async listRepositoryConnections(selection: PageInput, context: RequestContext): Promise<RepositoryConnectionPageDto> {
    const response = await this.#unary<ListRepositoryConnectionsResponse>(context, (metadata, options, callback) =>
      this.#repositories.listRepositoryConnections({ page: page(selection) }, metadata, options, callback));
    return {
      repositoryConnections: response.repositoryConnections.map(mapRepositoryConnection),
      page: pageDto(response.page),
    };
  }

  async getRepositoryConnection(repositoryConnectionId: string, context: RequestContext): Promise<RepositoryConnectionDto> {
    const response = await this.#unary<GetRepositoryConnectionResponse>(context, (metadata, options, callback) =>
      this.#repositories.getRepositoryConnection({ repositoryConnectionId }, metadata, options, callback));
    return mapRepositoryConnection(required(response.repositoryConnection));
  }

  async createRepositoryConnection(input: CreateRepositoryConnectionInput, context: RequestContext): Promise<RepositoryConnectionDto> {
    const response = await this.#unary<CreateRepositoryConnectionResponse>(context, (metadata, options, callback) =>
      this.#repositories.createRepositoryConnection({
        idempotencyKey: input.idempotencyKey,
        provider: toProtoRepositoryProvider(input),
        displayName: input.displayName,
        defaultBranch: input.defaultBranch,
      }, metadata, options, callback));
    return mapRepositoryConnection(required(response.repositoryConnection));
  }

  async listRuntimeProfiles(repositoryConnectionId: string | undefined, selection: PageInput, context: RequestContext): Promise<RuntimeProfilePageDto> {
    const response = await this.#unary<ListRuntimeProfilesResponse>(context, (metadata, options, callback) =>
      this.#profiles.listRuntimeProfiles({
        ...(repositoryConnectionId === undefined ? {} : { repositoryConnectionId }),
        page: page(selection),
      }, metadata, options, callback));
    return { runtimeProfiles: response.runtimeProfiles.map(mapRuntimeProfile), page: pageDto(response.page) };
  }

  async getRuntimeProfile(runtimeProfileId: string, context: RequestContext): Promise<RuntimeProfileDto> {
    const response = await this.#unary<GetRuntimeProfileResponse>(context, (metadata, options, callback) =>
      this.#profiles.getRuntimeProfile({ runtimeProfileId }, metadata, options, callback));
    return mapRuntimeProfile(required(response.runtimeProfile));
  }

  async createRuntimeProfile(input: CreateRuntimeProfileInput, context: RequestContext): Promise<RuntimeProfileDto> {
    const response = await this.#unary<CreateRuntimeProfileResponse>(context, (metadata, options, callback) =>
      this.#profiles.createRuntimeProfile({
        idempotencyKey: input.idempotencyKey,
        repositoryConnectionId: input.repositoryConnectionId,
        repositorySnapshotId: input.repositorySnapshotId,
        configuration: toProtoProfileConfiguration(input.configuration),
      }, metadata, options, callback));
    return mapRuntimeProfile(required(response.runtimeProfile));
  }

  async listRuntimeArtifacts(runtimeProfileId: string, selection: PageInput, context: RequestContext): Promise<RuntimeArtifactPageDto> {
    const response = await this.#unary<ListRuntimeArtifactsResponse>(context, (metadata, options, callback) =>
      this.#profiles.listRuntimeArtifacts({ runtimeProfileId, page: page(selection) }, metadata, options, callback));
    return { runtimeArtifacts: response.runtimeArtifacts.map(mapRuntimeArtifact), page: pageDto(response.page) };
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
      page: pageDto(response.page),
    };
  }

  async listDevelopmentTasks(selection: PageInput, context: RequestContext): Promise<DevelopmentTaskPageDto> {
    const response = await this.#unary<ListDevelopmentTasksResponse>(context, (metadata, options, callback) =>
      this.#tasks.listDevelopmentTasks({ page: page(selection) }, metadata, options, callback));
    return {
      developmentTasks: response.developmentTasks.map(mapDevelopmentTask),
      page: pageDto(response.page),
    };
  }

  async getDevelopmentTask(developmentTaskId: string, context: RequestContext): Promise<DevelopmentTaskDto> {
    const response = await this.#unary<GetDevelopmentTaskResponse>(context, (metadata, options, callback) =>
      this.#tasks.getDevelopmentTask({ developmentTaskId }, metadata, options, callback));
    return mapDevelopmentTask(required(response.developmentTask));
  }

  async createDevelopmentTask(input: CreateDevelopmentTaskInput, context: RequestContext): Promise<DevelopmentTaskDto> {
    const response = await this.#unary<CreateDevelopmentTaskResponse>(context, (metadata, options, callback) =>
      this.#tasks.createDevelopmentTask({
        idempotencyKey: input.idempotencyKey,
        request: toProtoTaskRequest(input.request),
      }, metadata, options, callback));
    return mapDevelopmentTask(required(response.developmentTask));
  }

  async cancelDevelopmentTask(input: TaskActionInput, context: RequestContext): Promise<DevelopmentTaskDto> {
    const response = await this.#unary<CancelDevelopmentTaskResponse>(context, (metadata, options, callback) =>
      this.#tasks.cancelDevelopmentTask(input, metadata, options, callback));
    return mapDevelopmentTask(required(response.developmentTask));
  }

  async retryDevelopmentTask(input: TaskActionInput, context: RequestContext): Promise<DevelopmentAttemptDto> {
    const response = await this.#unary<RetryDevelopmentTaskResponse>(context, (metadata, options, callback) =>
      this.#tasks.retryDevelopmentTask(input, metadata, options, callback));
    return mapDevelopmentAttempt(required(response.attempt));
  }

  watchDevelopmentTask(
    developmentTaskId: string,
    afterCursor: string | undefined,
    context: RequestContext,
    onUpdate: (update: DevelopmentTaskUpdateDto) => void | Promise<void>,
  ): Promise<void> {
    const acceptedRequestId = requestId(context.requestId);
    const metadata = new Metadata();
    metadata.set("x-request-id", acceptedRequestId);
    const duration = Math.min(Math.max(context.deadlineMs ?? DEFAULT_STREAM_DEADLINE_MS, 100), DEFAULT_STREAM_DEADLINE_MS);
    const acceptedCursor = cursor(afterCursor);
    const stream = this.#tasks.watchDevelopmentTask({
      developmentTaskId,
      ...(acceptedCursor === undefined ? {} : { afterCursor: acceptedCursor }),
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
      const abort = (): void => { stream.cancel(); finish(); };
      if (context.signal?.aborted === true) { abort(); return; }
      context.signal?.addEventListener("abort", abort, { once: true });
      stream.on("data", (response: WatchDevelopmentTaskResponse) => {
        stream.pause();
        Promise.resolve().then(() => {
          const update = required(response.update);
          switch (update.$case) {
            case "developmentTask":
              return onUpdate({ kind: "development_task", cursor: response.cursor, developmentTask: mapDevelopmentTask(update.developmentTask) });
            case "attempt":
              return onUpdate({ kind: "attempt", cursor: response.cursor, attempt: mapDevelopmentAttempt(update.attempt) });
            case "candidate":
              return onUpdate({ kind: "candidate", cursor: response.cursor, candidateId: update.candidate.id });
          }
        }).then(() => stream.resume()).catch((error: unknown) => { stream.cancel(); finish(error); });
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
