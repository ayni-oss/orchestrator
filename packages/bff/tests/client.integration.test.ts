import { BinaryWriter } from "@bufbuild/protobuf/wire";
import {
  status as grpcStatus,
  Metadata,
  Server,
  ServerCredentials,
  type sendUnaryData,
  type ServerUnaryCall,
  type ServerWritableStream,
  type ServiceError,
  type UntypedServiceImplementation,
} from "@grpc/grpc-js";
import {
  DevelopmentTaskPhase,
  DevelopmentTaskServiceService,
  ErrorDetail,
  ErrorReason,
  RepositoryServiceService,
  type CreateDevelopmentTaskRequest,
  type CreateDevelopmentTaskResponse,
  type GetRepositoryRequest,
  type GetRepositoryResponse,
  type ListRepositoriesRequest,
  type ListRepositoriesResponse,
  type SaveRepositoryRequest,
  type SaveRepositoryResponse,
  type WatchDevelopmentTaskRequest,
  type WatchDevelopmentTaskResponse,
} from "@ayni-orchestrator/grpc-bindings/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BffError, OrchestratorBff } from "../src/index.js";

function richStatus(): Buffer {
  const detail = ErrorDetail.encode({
    reason: ErrorReason.ERROR_REASON_INVALID_INPUT,
    message: "safe validation failure",
    metadata: [],
    requestId: "integration-request",
  }).finish();
  const any = new BinaryWriter()
    .uint32(10).string("type.googleapis.com/ayni.orchestrator.v1.ErrorDetail")
    .uint32(18).bytes(detail)
    .finish();
  return Buffer.from(new BinaryWriter()
    .uint32(8).int32(grpcStatus.INVALID_ARGUMENT)
    .uint32(18).string("safe validation failure")
    .uint32(26).bytes(any)
    .finish());
}

function taskUpdate(): WatchDevelopmentTaskResponse {
  return {
    task: {
      id: "00000000-0000-4000-8000-000000000001",
      input: {
        repositoryId: "sandbox",
        model: "gpt-5",
        startRevision: "a".repeat(40),
        request: "Correct the formula",
        acceptanceCriteria: ["three becomes eight"],
        changeConstraints: { protectedPaths: [], allowTestEdits: false },
      },
      repository: {
        id: "sandbox",
        owner: "ayni-oss",
        repository: "sandbox",
        baseBranch: "main",
        lockSha256: "b".repeat(64),
      },
      model: { provider: "openai", model: "gpt-5" },
      createdAt: new Date("2026-01-01T00:00:00.000Z"),
      updatedAt: new Date("2026-01-01T00:00:01.000Z"),
      phase: DevelopmentTaskPhase.DEVELOPMENT_TASK_PHASE_QUEUED,
      scopeViolations: [],
      lifecycleComplete: false,
      lifecycle: [],
      modelUsage: {
        collectionStatus: 2,
        interactions: [],
        byStage: [],
        total: { interactionCount: 0, unavailableInteractionCount: 0 },
      },
      resourceVersion: "8",
    },
    resourceVersion: "8",
  };
}

describe("native gRPC BFF integration", () => {
  let server: Server;
  let bff: OrchestratorBff;
  let receivedRequestId: string | undefined;
  let receivedIdempotencyKey: string | undefined;
  let oversizedCreateReachedServer = false;
  let receivedStreamCursor: string | undefined;
  let hangingStreamStarted = false;
  let hangingStreamCancelled = false;

  beforeEach(async () => {
    receivedRequestId = undefined;
    receivedIdempotencyKey = undefined;
    oversizedCreateReachedServer = false;
    receivedStreamCursor = undefined;
    hangingStreamStarted = false;
    hangingStreamCancelled = false;
    server = new Server();
    const implementation: UntypedServiceImplementation = {
      listRepositories(
        call: ServerUnaryCall<ListRepositoriesRequest, ListRepositoriesResponse>,
        callback: sendUnaryData<ListRepositoriesResponse>,
      ): void {
        const value = call.metadata.get("x-request-id")[0];
        receivedRequestId = typeof value === "string" ? value : undefined;
        callback(null, {
          repositories: [{
            enrollment: {
              id: "sandbox",
              owner: call.request.page?.pageToken === "large" ? "x".repeat(4 * 1024 * 1024) : "ayni-oss",
              repository: "sandbox",
              installationId: "42",
              baseBranch: "main",
              lockSha256: "a".repeat(64),
            },
          }],
          page: { nextPageToken: "" },
        });
      },
      saveRepository(
        call: ServerUnaryCall<SaveRepositoryRequest, SaveRepositoryResponse>,
        callback: sendUnaryData<SaveRepositoryResponse>,
      ): void {
        receivedIdempotencyKey = call.request.idempotencyKey;
        callback(null, { repository: { enrollment: call.request.repository } });
      },
      getRepository(
        call: ServerUnaryCall<GetRepositoryRequest, GetRepositoryResponse>,
        callback: sendUnaryData<GetRepositoryResponse>,
      ): void {
        if (call.request.repositoryId === "hang") return;
        const metadata = new Metadata();
        metadata.set("grpc-status-details-bin", richStatus());
        callback(Object.assign(new Error("secret upstream diagnostic"), {
          code: grpcStatus.INVALID_ARGUMENT,
          details: "secret upstream diagnostic",
          metadata,
        }) as ServiceError, null);
      },
    };
    server.addService(RepositoryServiceService, implementation);
    server.addService(DevelopmentTaskServiceService, {
      createDevelopmentTask(
        _call: ServerUnaryCall<CreateDevelopmentTaskRequest, CreateDevelopmentTaskResponse>,
        callback: sendUnaryData<CreateDevelopmentTaskResponse>,
      ): void {
        oversizedCreateReachedServer = true;
        callback(null, { task: taskUpdate().task });
      },
      watchDevelopmentTask(
        call: ServerWritableStream<WatchDevelopmentTaskRequest, WatchDevelopmentTaskResponse>,
      ): void {
        receivedStreamCursor = call.request.afterResourceVersion;
        if (call.request.taskId === "hang") {
          hangingStreamStarted = true;
          call.on("cancelled", () => {
            hangingStreamCancelled = true;
          });
          return;
        }
        call.write(taskUpdate());
        call.end();
      },
    });
    const port = await new Promise<number>((resolve, reject) => {
      server.bindAsync("127.0.0.1:0", ServerCredentials.createInsecure(), (error, boundPort) => {
        if (error === null) resolve(boundPort);
        else reject(error);
      });
    });
    bff = new OrchestratorBff({ address: `127.0.0.1:${port}` });
  });

  afterEach(async () => {
    bff.close();
    await new Promise<void>((resolve) => server.tryShutdown(() => resolve()));
  });

  it("uses generated native clients, propagates correlation, and returns DTOs", async () => {
    const response = await bff.listRepositories({}, { requestId: "integration-request" });
    expect(receivedRequestId).toBe("integration-request");
    expect(response.repositories[0]).toEqual(expect.objectContaining({
      enrollment: expect.objectContaining({ id: "sandbox", installationId: "42" }),
    }));
  });

  it("preserves caller idempotency keys on mutations", async () => {
    await bff.saveRepository({
      idempotencyKey: "browser-save-1",
      repository: {
        id: "sandbox",
        owner: "ayni-oss",
        repository: "sandbox",
        installationId: "42",
        baseBranch: "main",
        lockSha256: "a".repeat(64),
      },
    }, { requestId: "mutation-request" });
    expect(receivedIdempotencyKey).toBe("browser-save-1");
  });

  it("maps rich gRPC errors without exposing raw transport diagnostics", async () => {
    const result = bff.getRepository("invalid", { requestId: "integration-request" });
    await expect(result).rejects.toMatchObject({
      status: 400,
      reason: "invalid_input",
      message: "safe validation failure",
      requestId: "integration-request",
    } satisfies Partial<BffError>);
    await expect(result).rejects.not.toHaveProperty("message", "secret upstream diagnostic");
  });

  it("applies bounded deadlines", async () => {
    const result = bff.getRepository("hang", {
      requestId: "deadline-request",
      deadlineMs: 100,
    });
    await expect(result).rejects.toMatchObject({
      status: 504,
      retryable: true,
    } satisfies Partial<BffError>);
  });

  it("cancels unary gRPC calls when the browser request aborts", async () => {
    const controller = new AbortController();
    const result = bff.getRepository("hang", {
      requestId: "cancel-request",
      signal: controller.signal,
      deadlineMs: 5_000,
    });
    controller.abort();
    await expect(result).rejects.toMatchObject({
      status: 499,
      reason: "cancelled",
    } satisfies Partial<BffError>);
  });

  it("enforces the four MiB response limit", async () => {
    const result = bff.listRepositories({ pageToken: "large" }, { requestId: "large-response" });
    await expect(result).rejects.toMatchObject({ status: 429 } satisfies Partial<BffError>);
  });

  it("enforces the one MiB request limit before server dispatch", async () => {
    const result = bff.createDevelopmentTask({
      idempotencyKey: "large-request",
      input: {
        repositoryId: "sandbox",
        model: "gpt-5",
        startRevision: "a".repeat(40),
        request: "x".repeat(1_100_000),
        acceptanceCriteria: ["bounded result"],
        changeConstraints: { protectedPaths: [], allowTestEdits: false },
      },
    }, { requestId: "large-request" });
    await expect(result).rejects.toMatchObject({ status: 429 } satisfies Partial<BffError>);
    expect(oversizedCreateReachedServer).toBe(false);
  });

  it("rejects an overflowing stream cursor before serialization", () => {
    expect(() => bff.watchDevelopmentTask(
      "00000000-0000-4000-8000-000000000001",
      "18446744073709551616",
      { requestId: "stream-cursor-request" },
      () => undefined,
    )).toThrow(expect.objectContaining({ status: 400, reason: "invalid_input" }));
  });

  it("relays resumable server-streamed task DTOs", async () => {
    const updates: string[] = [];
    await bff.watchDevelopmentTask(
      "00000000-0000-4000-8000-000000000001",
      "7",
      { requestId: "stream-request", deadlineMs: 1_000 },
      (task) => {
        updates.push(`${task.id}:${task.resourceVersion}`);
      },
    );
    expect(receivedStreamCursor).toBe("7");
    expect(updates).toEqual(["00000000-0000-4000-8000-000000000001:8"]);
  });

  it("cancels an active gRPC stream when the browser aborts", async () => {
    const controller = new AbortController();
    const result = bff.watchDevelopmentTask(
      "hang",
      undefined,
      { requestId: "stream-cancel-request", signal: controller.signal, deadlineMs: 1_000 },
      () => undefined,
    );
    await vi.waitFor(() => expect(hangingStreamStarted).toBe(true));
    controller.abort();
    await expect(result).resolves.toBeUndefined();
    await vi.waitFor(() => expect(hangingStreamCancelled).toBe(true));
  });

  it("applies a bounded deadline to an active gRPC stream", async () => {
    const result = bff.watchDevelopmentTask(
      "hang",
      undefined,
      { requestId: "stream-deadline-request", deadlineMs: 100 },
      () => undefined,
    );
    await expect(result).rejects.toMatchObject({
      status: 504,
      reason: "deadline_exceeded",
      retryable: true,
    } satisfies Partial<BffError>);
    await vi.waitFor(() => expect(hangingStreamCancelled).toBe(true));
  });

  it("turns a stream consumer exception into a safe rejection", async () => {
    const result = bff.watchDevelopmentTask(
      "00000000-0000-4000-8000-000000000001",
      undefined,
      { requestId: "stream-error-request", deadlineMs: 1_000 },
      () => {
        throw new Error("sensitive browser consumer state");
      },
    );
    await expect(result).rejects.toMatchObject({
      status: 500,
      reason: "internal",
      message: "the browser server could not complete the request",
    } satisfies Partial<BffError>);
  });

  it("rejects non-loopback backends in the trusted profile", () => {
    expect(() => new OrchestratorBff({ address: "control-plane:50051" })).toThrow(/loopback/u);
  });
});
