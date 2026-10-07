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
  DevelopmentTaskServiceService,
  DevelopmentTaskState,
  ErrorDetail,
  ErrorReason,
  RepositoryConnectionStatus,
  RepositoryServiceService,
  RuntimeProfileKind,
  type CreateDevelopmentTaskRequest,
  type CreateDevelopmentTaskResponse,
  type CreateRepositoryConnectionRequest,
  type CreateRepositoryConnectionResponse,
  type GetRepositoryConnectionRequest,
  type GetRepositoryConnectionResponse,
  type ListRepositoryConnectionsRequest,
  type ListRepositoryConnectionsResponse,
  type WatchDevelopmentTaskRequest,
  type WatchDevelopmentTaskResponse,
} from "@ayni-orchestrator/grpc-bindings/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BffError, OrchestratorBff, toBffError } from "../src/index.js";

function richStatus(reason = ErrorReason.ERROR_REASON_INVALID_INPUT): Buffer {
  const detail = ErrorDetail.encode({
    reason,
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

function task() {
  return {
    id: "task:1",
    projectId: "project:1",
    snapshot: { limits:{codingTurns:24,qualityRepairLimit:3}, publicationBaseBranch:"main",
      repository: {
        repositoryConnectionId: "repository:1",
        repositorySnapshotId: "snapshot:1",
        revision: "a".repeat(40),
      },
      runtime: {
        runtimeProfileId: "profile:1",
        runtimeProfileRevisionId: "profile-revision:1",
        profileKind: RuntimeProfileKind.RUNTIME_PROFILE_KIND_AYNI_MANAGED,
        runtimeArtifactId: "artifact:1",
        ociRepository: "ghcr.io/ayni-oss/sandbox/ayni-environment",
        resolvedDigest: `sha256:${"b".repeat(64)}`,
        platform: { operatingSystem: "linux", architecture: "amd64" },
      },
      agent: { provider: "openai", agent: "coding-agent", model: "gpt-5" },
      request: "Correct the formula",
      acceptanceCriteria: ["three becomes eight"],
      changeConstraints: { protectedPaths: [], allowTestEdits: false },
    },
    state: DevelopmentTaskState.DEVELOPMENT_TASK_STATE_QUEUED,
    attemptCount: 1,
    createdAt: new Date("2026-01-01T00:00:00.000Z"),
    updatedAt: new Date("2026-01-01T00:00:01.000Z"),
    resourceVersion: "8",
  };
}

function taskUpdate(): WatchDevelopmentTaskResponse {
  return {
    cursor: "8",
    update: { $case: "developmentTask", developmentTask: task() },
  };
}

describe("clean-v1 native gRPC BFF integration", () => {
  it("maps every stable rich error reason and fails closed for unknown values", () => {
    const cases: Array<[ErrorReason, string]> = [
      [ErrorReason.ERROR_REASON_UNSPECIFIED, "unspecified"],
      [ErrorReason.ERROR_REASON_INVALID_IDENTIFIER, "invalid_identifier"],
      [ErrorReason.ERROR_REASON_INVALID_INPUT, "invalid_input"],
      [ErrorReason.ERROR_REASON_IDEMPOTENCY_KEY_REUSED, "idempotency_key_reused"],
      [ErrorReason.ERROR_REASON_RESOURCE_NOT_FOUND, "resource_not_found"],
      [ErrorReason.ERROR_REASON_CONFLICT, "conflict"],
      [ErrorReason.ERROR_REASON_FAILED_PRECONDITION, "failed_precondition"],
      [ErrorReason.ERROR_REASON_PROVIDER_UNAVAILABLE, "provider_unavailable"],
      [ErrorReason.ERROR_REASON_MODEL_UNAVAILABLE, "model_unavailable"],
      [ErrorReason.ERROR_REASON_REPOSITORY_ACCESS, "repository_access"],
      [ErrorReason.ERROR_REASON_CANCELLED, "cancelled"],
      [ErrorReason.ERROR_REASON_DEADLINE_EXCEEDED, "deadline_exceeded"],
      [ErrorReason.ERROR_REASON_INTERNAL, "internal"],
      [ErrorReason.ERROR_REASON_INVALID_REPOSITORY_REVISION, "invalid_repository_revision"],
      [ErrorReason.ERROR_REASON_INVALID_PROFILE_REVISION, "invalid_profile_revision"],
      [ErrorReason.ERROR_REASON_INVALID_LOCK_FINGERPRINT, "invalid_lock_fingerprint"],
      [ErrorReason.ERROR_REASON_INVALID_OCI_REFERENCE, "invalid_oci_reference"],
      [ErrorReason.ERROR_REASON_INVALID_OCI_DIGEST, "invalid_oci_digest"],
      [ErrorReason.ERROR_REASON_INVALID_PLATFORM, "invalid_platform"],
      [ErrorReason.ERROR_REASON_PROFILE_UNSUPPORTED, "profile_unsupported"],
      [ErrorReason.ERROR_REASON_PROFILE_NOT_READY, "profile_not_ready"],
      [ErrorReason.ERROR_REASON_RUNTIME_ARTIFACT_NOT_VERIFIED, "runtime_artifact_not_verified"],
      [ErrorReason.ERROR_REASON_AUTOMATION_NOT_READY, "automation_not_ready"],
      [ErrorReason.ERROR_REASON_TASK_NOT_ACTIONABLE, "task_not_actionable"],
      [ErrorReason.ERROR_REASON_TASK_DEADLINE_EXPIRED, "task_deadline_expired"],
      [ErrorReason.ERROR_REASON_BOUND_EXCEEDED, "bound_exceeded"],
      [ErrorReason.ERROR_REASON_WATCH_CURSOR_EXPIRED, "watch_cursor_expired"],
      [ErrorReason.ERROR_REASON_UNKNOWN_ENUM_VALUE, "unknown_enum_value"],
      [ErrorReason.ERROR_REASON_NOT_IMPLEMENTED, "not_implemented"],
      [ErrorReason.UNRECOGNIZED, "unspecified"],
    ];
    for (const [value, expected] of cases) {
      const metadata = new Metadata();
      metadata.set("grpc-status-details-bin", richStatus(value));
      const error = Object.assign(new Error("unsafe transport message"), {
        code: grpcStatus.INVALID_ARGUMENT,
        details: "unsafe transport message",
        metadata,
      }) as ServiceError;
      expect(toBffError(error, "fallback-request").reason).toBe(expected);
    }
  });

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
      listRepositoryConnections(
        call: ServerUnaryCall<ListRepositoryConnectionsRequest, ListRepositoryConnectionsResponse>,
        callback: sendUnaryData<ListRepositoryConnectionsResponse>,
      ): void {
        const value = call.metadata.get("x-request-id")[0];
        receivedRequestId = typeof value === "string" ? value : undefined;
        callback(null, {
          repositoryConnections: [{
            id: "repository:1",
            provider: {
              provider: {
                $case: "github",
                github: {
                  repositoryId: "123",
                  owner: call.request.page?.pageToken === "large" ? "x".repeat(4 * 1024 * 1024) : "ayni-oss",
                  name: "sandbox",
                  installationId: "42",
                  htmlUrl: "https://github.com/ayni-oss/sandbox",
                },
              },
            },
            displayName: "ayni-oss/sandbox",
            defaultBranch: "main",
            status: RepositoryConnectionStatus.REPOSITORY_CONNECTION_STATUS_CONNECTED,
            createdAt: new Date("2026-01-01T00:00:00.000Z"),
            updatedAt: new Date("2026-01-01T00:00:01.000Z"),
            resourceVersion: "1",
          }],
          page: { nextPageToken: "", snapshotCursor: "1" },
        });
      },
      createRepositoryConnection(
        call: ServerUnaryCall<CreateRepositoryConnectionRequest, CreateRepositoryConnectionResponse>,
        callback: sendUnaryData<CreateRepositoryConnectionResponse>,
      ): void {
        receivedIdempotencyKey = call.request.idempotencyKey;
        const provider = call.request.provider;
        callback(null, {
          repositoryConnection: {
            id: "repository:1",
            provider,
            displayName: call.request.displayName,
            defaultBranch: call.request.defaultBranch,
            status: RepositoryConnectionStatus.REPOSITORY_CONNECTION_STATUS_CONNECTING,
            createdAt: new Date("2026-01-01T00:00:00.000Z"),
            updatedAt: new Date("2026-01-01T00:00:00.000Z"),
            resourceVersion: "1",
          },
        });
      },
      getRepositoryConnection(
        call: ServerUnaryCall<GetRepositoryConnectionRequest, GetRepositoryConnectionResponse>,
        callback: sendUnaryData<GetRepositoryConnectionResponse>,
      ): void {
        if (call.request.repositoryConnectionId === "hang") return;
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
        callback(null, { developmentTask: task() });
      },
      watchDevelopmentTask(
        call: ServerWritableStream<WatchDevelopmentTaskRequest, WatchDevelopmentTaskResponse>,
      ): void {
        receivedStreamCursor = call.request.afterCursor;
        if (call.request.developmentTaskId === "hang") {
          hangingStreamStarted = true;
          call.on("cancelled", () => { hangingStreamCancelled = true; });
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

  it("uses generated native clients and returns provider-neutral DTOs", async () => {
    const response = await bff.listRepositoryConnections({}, { requestId: "integration-request" });
    expect(receivedRequestId).toBe("integration-request");
    expect(response.repositoryConnections[0]).toEqual(expect.objectContaining({
      id: "repository:1",
      provider: expect.objectContaining({ kind: "github", installationId: "42" }),
    }));
  });

  it("preserves caller idempotency keys on mutations", async () => {
    await bff.createRepositoryConnection({
      idempotencyKey: "browser-create-1",
      provider: {
        kind: "github",
        repositoryId: "123",
        owner: "ayni-oss",
        name: "sandbox",
        installationId: "42",
        htmlUrl: "https://github.com/ayni-oss/sandbox",
      },
      displayName: "ayni-oss/sandbox",
      defaultBranch: "main",
    }, { requestId: "mutation-request" });
    expect(receivedIdempotencyKey).toBe("browser-create-1");
  });

  it("maps rich gRPC errors without exposing raw transport diagnostics", async () => {
    const result = bff.getRepositoryConnection("invalid", { requestId: "integration-request" });
    await expect(result).rejects.toMatchObject({
      status: 400,
      reason: "invalid_input",
      message: "safe validation failure",
      requestId: "integration-request",
    } satisfies Partial<BffError>);
    await expect(result).rejects.not.toHaveProperty("message", "secret upstream diagnostic");
  });

  it("applies deadlines and response bounds", async () => {
    await expect(bff.getRepositoryConnection("hang", {
      requestId: "deadline-request",
      deadlineMs: 100,
    })).rejects.toMatchObject({ status: 504, retryable: true } satisfies Partial<BffError>);
    await expect(bff.listRepositoryConnections({ pageToken: "large" }, { requestId: "large" }))
      .rejects.toMatchObject({ status: 429 } satisfies Partial<BffError>);
  });

  it("enforces the one MiB request limit before server dispatch", async () => {
    const result = bff.createDevelopmentTask({
      idempotencyKey: "large-request",
      request: {
        repositoryConnectionId: "repository:1",
        projectId: "project:1",
      repositoryRevision: "a".repeat(40),
        runtimeProfileId: "profile:1",
        agent: { provider: "openai", agent: "coding-agent", model: "gpt-5" },
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
      "task:1",
      "18446744073709551616",
      { requestId: "stream-cursor-request" },
      () => undefined,
    )).toThrow(expect.objectContaining({ status: 400, reason: "invalid_input" }));
  });

  it("relays resumable typed task updates", async () => {
    const updates: string[] = [];
    await bff.watchDevelopmentTask("task:1", "7", { requestId: "stream", deadlineMs: 1_000 }, (update) => {
      if (update.kind === "development_task") updates.push(`${update.developmentTask.id}:${update.cursor}`);
    });
    expect(receivedStreamCursor).toBe("7");
    expect(updates).toEqual(["task:1:8"]);
  });

  it("cancels active streams and sanitizes consumer failures", async () => {
    const controller = new AbortController();
    const result = bff.watchDevelopmentTask(
      "hang", undefined, { requestId: "cancel", signal: controller.signal, deadlineMs: 1_000 }, () => undefined,
    );
    await vi.waitFor(() => expect(hangingStreamStarted).toBe(true));
    controller.abort();
    await expect(result).resolves.toBeUndefined();
    await vi.waitFor(() => expect(hangingStreamCancelled).toBe(true));

    await expect(bff.watchDevelopmentTask(
      "task:1", undefined, { requestId: "consumer", deadlineMs: 1_000 }, () => {
        throw new Error("sensitive browser state");
      },
    )).rejects.toMatchObject({ status: 500, reason: "internal" } satisfies Partial<BffError>);
  });

  it("rejects non-loopback backends", () => {
    expect(() => new OrchestratorBff({ address: "control-plane:50051" })).toThrow(/loopback/u);
  });
});
