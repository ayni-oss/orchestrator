import {
  credentials,
  Metadata,
  type CallOptions,
  type ClientOptions,
  type ClientUnaryCall,
  type ServiceError,
} from "@grpc/grpc-js";
import {
  ProjectServiceClient, ProjectState,
  type Project, type CreateProjectResponse, type GetProjectResponse, type ListProjectsResponse, type UpdateProjectResponse, type ArchiveProjectResponse,
  DevelopmentTaskServiceClient,
  ReviewServiceClient,
  type InspectAyniRepositoryResponse, type GetRuntimeProfileRevisionResponse,
  type ListAutomationInstallationsResponse, type InstallRepositoryAutomationResponse,
  type ReconcileRuntimeProfileResponse, type ListCandidatesResponse,
  type ListDevelopmentAttemptsResponse, type PublishCandidateResponse,

  type ListChangeRequestsResponse,
  type GetChangeRequestResponse,
  type ListAssessmentsResponse,
  type ListEvidenceReferencesResponse,
  ProviderServiceClient,
  RepositoryServiceClient,
  RuntimeProfileServiceClient,
  type CancelDevelopmentTaskResponse,
  type CreateDevelopmentTaskResponse,
  type CreateRepositoryConnectionResponse,
  type ConnectGithubRepositoryResponse,
  type CreateRuntimeProfileResponse,
  type GetDevelopmentTaskResponse,
  type GetRepositoryConnectionResponse,
  type GetRuntimeProfileResponse,
  type ListDevelopmentTasksResponse,
  type ListProviderModelsResponse,
  type ListRepositoryConnectionsResponse,
  type ListGithubInstallationsResponse,
  type ListGithubInstallationRepositoriesResponse,
  type ListRuntimeArtifactsResponse,
  type ListRuntimeProfilesResponse,
  type RetryDevelopmentTaskResponse,
  type WatchDevelopmentTaskResponse,
} from "@ayni-orchestrator/grpc-bindings/server";
import { mapInspection, mapRevision, mapAutomation, mapCandidate, mapAttemptUsage } from "./onboarding.js";
import { mapReview, mapAssessment, mapEvidence } from "./reviews.js";
import { randomUUID } from "node:crypto";
import type {
  ProjectDto,
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
  GithubInstallationOptionDto,
  GithubRepositoryOptionDto,
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

function mapProject(value: Project): ProjectDto {
  if (value.repositoryConnectionIds.length < 1 || value.repositoryConnectionIds.length > 100 || new Set(value.repositoryConnectionIds).size !== value.repositoryConnectionIds.length || ![ProjectState.PROJECT_STATE_ACTIVE, ProjectState.PROJECT_STATE_ARCHIVED].includes(value.state)) throw new BffMappingError();
  return {id:value.id, displayName:value.displayName, state:value.state === ProjectState.PROJECT_STATE_ACTIVE ? "active" : "archived", repositoryConnectionIds:[...value.repositoryConnectionIds], resourceVersion:value.resourceVersion};
}

export class OrchestratorBff {
  readonly #projects: ProjectServiceClient;
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
    this.#projects = new ProjectServiceClient(address, channelCredentials, options);
    this.#repositories = new RepositoryServiceClient(address, channelCredentials, options);
    this.#profiles = new RuntimeProfileServiceClient(address, channelCredentials, options);
    this.#providers = new ProviderServiceClient(address, channelCredentials, options);
    this.#tasks = new DevelopmentTaskServiceClient(address, channelCredentials, options);
    this.#reviews = new ReviewServiceClient(address, channelCredentials, options);
  }

  async listProjects(selection: PageInput, context: RequestContext, activeOnly = false) {
    const response = await this.#unary<ListProjectsResponse>(context, (m,o,c) => this.#projects.listProjects({page:page(selection), ...(activeOnly ? {state:ProjectState.PROJECT_STATE_ACTIVE} : {})},m,o,c));
    return {projects:response.projects.map(mapProject), page:pageDto(response.page)};
  }
  async getProject(projectId:string, context:RequestContext):Promise<ProjectDto> {
    const response = await this.#unary<GetProjectResponse>(context,(m,o,c)=>this.#projects.getProject({projectId},m,o,c));
    return mapProject(required(response.project));
  }
  async createProject(input:{idempotencyKey:string;displayName:string;repositoryConnectionIds:string[]}, context:RequestContext):Promise<ProjectDto> {
    const response = await this.#unary<CreateProjectResponse>(context,(m,o,c)=>this.#projects.createProject({...input,repositoryConnectionId:""},m,o,c));
    return mapProject(required(response.project));
  }
  async updateProject(input:{idempotencyKey:string;projectId:string;expectedResourceVersion:string;displayName:string;repositoryConnectionIds?:string[]}, context:RequestContext):Promise<ProjectDto> {
    const response = await this.#unary<UpdateProjectResponse>(context,(m,o,c)=>this.#projects.updateProject({...input,repositoryConnectionIds:input.repositoryConnectionIds ?? []},m,o,c));
    return mapProject(required(response.project));
  }
  async archiveProject(input:{idempotencyKey:string;projectId:string;expectedResourceVersion:string}, context:RequestContext):Promise<ProjectDto> {
    const response = await this.#unary<ArchiveProjectResponse>(context,(m,o,c)=>this.#projects.archiveProject(input,m,o,c));
    return mapProject(required(response.project));
  }

  close(): void {
    this.#projects.close();
    this.#repositories.close();
    this.#profiles.close();
    this.#providers.close();
    this.#tasks.close();
    this.#reviews.close();
  }


  async connectGithubRepository(input: { idempotencyKey:string; owner:string; name:string }, context: RequestContext) {
    const response = await this.#unary<ConnectGithubRepositoryResponse>(context, (m,o,c)=>this.#repositories.connectGithubRepository(input,m,o,c));
    return mapRepositoryConnection(required(response.repositoryConnection));
  }
  async listGithubInstallations(context: RequestContext): Promise<GithubInstallationOptionDto[]> {
    const response = await this.#unary<ListGithubInstallationsResponse>(context, (m,o,c)=>this.#repositories.listGithubInstallations({},m,o,c));
    return response.installations.map(item=>({installationId:item.installationId,owner:item.owner,appName:item.appName}));
  }
  async listGithubInstallationRepositories(installationId: string, context: RequestContext): Promise<GithubRepositoryOptionDto[]> {
    const response = await this.#unary<ListGithubInstallationRepositoriesResponse>(context, (m,o,c)=>this.#repositories.listGithubInstallationRepositories({installationId},m,o,c));
    return response.repositories.map(item=>({repositoryId:item.repositoryId,owner:item.owner,name:item.name,defaultBranch:item.defaultBranch}));
  }
  async inspectAyniRepository(repositoryConnectionId:string, context:RequestContext) {
    const response=await this.#unary<InspectAyniRepositoryResponse>(context,(m,o,c)=>this.#profiles.inspectAyniRepository({repositoryConnectionId},m,o,c));
    return mapInspection(response);
  }
  async getRuntimeRevision(runtimeProfileRevisionId:string, context:RequestContext) {
    const response=await this.#unary<GetRuntimeProfileRevisionResponse>(context,(m,o,c)=>this.#profiles.getRuntimeProfileRevision({runtimeProfileRevisionId},m,o,c));
    return mapRevision(required(response.revision));
  }
  async listAutomation(runtimeProfileId:string, context:RequestContext) {
    const response=await this.#unary<ListAutomationInstallationsResponse>(context,(m,o,c)=>this.#profiles.listAutomationInstallations({runtimeProfileId,page:page({pageSize:100})},m,o,c));
    return { installations:response.automationInstallations.map(mapAutomation), page:pageDto(response.page) };
  }
  async installAutomation(runtimeProfileId:string, idempotencyKey:string, context:RequestContext) {
    const response=await this.#unary<InstallRepositoryAutomationResponse>(context,(m,o,c)=>this.#profiles.installRepositoryAutomation({runtimeProfileId,idempotencyKey},m,o,c));
    return mapAutomation(required(response.automationInstallation));
  }
  async reconcileProfile(runtimeProfileId:string, idempotencyKey:string, context:RequestContext) {
    const response=await this.#unary<ReconcileRuntimeProfileResponse>(context,(m,o,c)=>this.#profiles.reconcileRuntimeProfile({runtimeProfileId,idempotencyKey},m,o,c));
    return mapRuntimeProfile(required(response.runtimeProfile));
  }
  async taskProgress(developmentTaskId:string, context:RequestContext) {
    const attempts=await this.#unary<ListDevelopmentAttemptsResponse>(context,(m,o,c)=>this.#tasks.listDevelopmentAttempts({developmentTaskId,page:page({pageSize:100})},m,o,c));
    const candidates=await this.#unary<ListCandidatesResponse>(context,(m,o,c)=>this.#tasks.listCandidates({developmentTaskId,page:page({pageSize:100})},m,o,c));
    const reviews=await Promise.all(candidates.candidates.map(candidate=>this.#unary<ListChangeRequestsResponse>(context,(m,o,c)=>this.#reviews.listChangeRequests({candidateId:candidate.id,page:page({pageSize:100})},m,o,c))));
    return {attempts:attempts.attempts.map(a=>({...mapDevelopmentAttempt(a),usage:mapAttemptUsage(a)})), candidates:candidates.candidates.map(mapCandidate), reviews:reviews.flatMap(r=>r.changeRequests.map(mapReview))};
  }
  async publishCandidate(input: {candidateId:string;idempotencyKey:string;title:string;body:string;baseBranch:string},context:RequestContext) {
    const response=await this.#unary<PublishCandidateResponse>(context,(m,o,c)=>this.#reviews.publishCandidate({...input,draft:true},m,o,c));
    return mapReview(required(response.changeRequest));
  }
  async getReview(changeRequestId:string,context:RequestContext) {
    const response=await this.#unary<GetChangeRequestResponse>(context,(m,o,c)=>this.#reviews.getChangeRequest({changeRequestId},m,o,c));
    return mapReview(required(response.changeRequest));
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

  async listDevelopmentTasks(selection: PageInput, context: RequestContext, projectId?: string): Promise<DevelopmentTaskPageDto> {
    const response = await this.#unary<ListDevelopmentTasksResponse>(context, (metadata, options, callback) =>
      this.#tasks.listDevelopmentTasks({ page: page(selection), ...(projectId ? {projectId} : {}) }, metadata, options, callback));
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
