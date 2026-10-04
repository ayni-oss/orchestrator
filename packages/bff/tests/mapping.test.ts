import {
  DevelopmentAttempt,
  DevelopmentAttemptState,
  DevelopmentTaskState,
  RepositoryConnectionStatus,
  RuntimeArtifactStatus,
  RuntimeProfileKind,
  RuntimeProfileState,
  RuntimeProfileSupport,
  type DevelopmentTask,
  type RepositoryConnection,
  type RuntimeProfile,
} from "@ayni-orchestrator/grpc-bindings/server";
import { mapDevelopmentAttempt } from "../src/mapping.js";
import { describe, expect, it } from "vitest";
import {
  BffMappingError,
  mapAttemptState,
  mapDevelopmentTask,
  mapRepositoryConnection,
  mapRepositoryStatus,
  mapRuntimeArtifactStatus,
  mapRuntimeProfile,
  mapRuntimeProfileKind,
  mapRuntimeProfileState,
  mapRuntimeProfileSupport,
  mapTaskState,
  toProtoProfileConfiguration,
  toProtoTaskRequest,
} from "../src/index.js";

function repository(): RepositoryConnection {
  return {
    id: "repository:1",
    provider: {
      provider: {
        $case: "github",
        github: {
          repositoryId: "123",
          owner: "ayni-oss",
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
    resourceVersion: "7",
  };
}

function profile(): RuntimeProfile {
  return {
    reconciliationMetadata: [],
    id: "profile:1",
    repositoryConnectionId: "repository:1",
    kind: RuntimeProfileKind.RUNTIME_PROFILE_KIND_AYNI_MANAGED,
    support: RuntimeProfileSupport.RUNTIME_PROFILE_SUPPORT_OPERATIONAL,
    state: RuntimeProfileState.RUNTIME_PROFILE_STATE_AWAITING_ENVIRONMENT,
    activeRevisionId: "profile-revision:1",
    createdAt: new Date("2026-01-01T00:00:00.000Z"),
    updatedAt: new Date("2026-01-01T00:00:01.000Z"),
    resourceVersion: "3",
  };
}

function task(): DevelopmentTask {
  return {
    id: "task:1",
    projectId: "project:1",
    snapshot: { publishDraft:false, publicationBaseBranch:"main",
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
      changeConstraints: { protectedPaths: ["docs"], allowTestEdits: false },
      deadlineAt: new Date("2026-01-01T01:00:00.000Z"),
    },
    state: DevelopmentTaskState.DEVELOPMENT_TASK_STATE_QUEUED,
    attemptCount: 1,
    createdAt: new Date("2026-01-01T00:00:00.000Z"),
    updatedAt: new Date("2026-01-01T00:00:01.000Z"),
    resourceVersion: "8",
  };
}

describe("clean-v1 browser DTO mappings", () => {
  it("maps provider boundaries without leaking transport objects", () => {
    const mapped = mapRepositoryConnection(repository());
    expect(mapped).toEqual(expect.objectContaining({
      id: "repository:1",
      provider: expect.objectContaining({ kind: "github", installationId: "42" }),
      status: "connected",
    }));
    expect(mapped).not.toHaveProperty("$type");
    expect(JSON.parse(JSON.stringify(mapped))).toEqual(mapped);
  });

  it("maps profile readiness and frozen task identities independently", () => {
    expect(mapRuntimeProfile(profile())).toEqual(expect.objectContaining({
      kind: "ayni_managed",
      state: "awaiting_environment",
      activeRevisionId: "profile-revision:1",
    }));
    expect(mapDevelopmentTask(task())).toEqual(expect.objectContaining({
      state: "queued",
      snapshot: expect.objectContaining({
        runtime: expect.objectContaining({ resolvedDigest: `sha256:${"b".repeat(64)}` }),
      }),
    }));
  });

  it("handles every additive enum boundary as unknown", () => {
    expect(mapRepositoryStatus(999 as RepositoryConnectionStatus)).toBe("unknown");
    expect(mapRuntimeProfileKind(999 as RuntimeProfileKind)).toBe("unknown");
    expect(mapRuntimeProfileState(999 as RuntimeProfileState)).toBe("unknown");
    expect(mapRuntimeProfileSupport(999 as RuntimeProfileSupport)).toBe("unknown");
    expect(mapRuntimeArtifactStatus(999 as RuntimeArtifactStatus)).toBe("unknown");
    expect(mapTaskState(999 as DevelopmentTaskState)).toBe("unknown");
    expect(mapAttemptState(999 as DevelopmentAttemptState)).toBe("unknown");
  });

  it("rejects incomplete required transport values", () => {
    expect(() => mapRepositoryConnection({ ...repository(), provider: undefined })).toThrow(BffMappingError);
    expect(() => mapRuntimeProfile({ ...profile(), createdAt: undefined })).toThrow(BffMappingError);
    expect(() => mapDevelopmentTask({ ...task(), snapshot: undefined })).toThrow(BffMappingError);
  });

  it("builds both closed profile variants without cross-profile fields", () => {
    const ayni = toProtoProfileConfiguration({
      kind: "ayni_managed",
      lockFingerprint: "a".repeat(64),
      ayniVersion: "0.13.0",
      workflowTemplateVersion: "v1",
      ociRepository: "ghcr.io/ayni-oss/sandbox/ayni-environment",
    });
    const custom = toProtoProfileConfiguration({
      kind: "custom_runtime",
      imageReference: "registry.example/runtime:stable",
    });
    expect(ayni.configuration?.$case).toBe("ayniManaged");
    expect(custom.configuration?.$case).toBe("customRuntime");
    expect(JSON.stringify(custom)).not.toContain("lockFingerprint");
    expect(JSON.stringify(custom)).not.toContain("workflowTemplateVersion");
  });

  it("freezes explicit agent, model, profile, request, constraints, and deadline input", () => {
    expect(toProtoTaskRequest({
      repositoryConnectionId: "repository:1",
      projectId: "project:1",
      repositoryRevision: "a".repeat(40),
      runtimeProfileId: "profile:1",
      agent: { provider: "openai", agent: "coding-agent", model: "gpt-5" },
      request: "Correct the formula",
      acceptanceCriteria: ["three becomes eight"],
      changeConstraints: { protectedPaths: ["docs"], allowTestEdits: false },
      deadlineSeconds: 60,
    })).toEqual(expect.objectContaining({
      runtimeProfileId: "profile:1",
      agent: { provider: "openai", agent: "coding-agent", model: "gpt-5" },
      deadline: { seconds: "60", nanos: 0 },
    }));
  });
});


it("retains bounded activity on failed attempts for task polling and watch updates", () => {
  const original = task();
  const attempt = DevelopmentAttempt.fromPartial({
    id: "attempt:1", developmentTaskId: original.id, snapshot: original.snapshot,
    state: DevelopmentAttemptState.DEVELOPMENT_ATTEMPT_STATE_FAILED,
    createdAt: original.createdAt, updatedAt: original.updatedAt,
    agentActivity: Array.from({ length: 300 }, (_, index) => ({
      sequence: index + 1, observedAt: original.createdAt,
      message: "Waiting for a model response", failed: index === 299,
    })),
  });
  const mapped = mapDevelopmentAttempt(attempt);
  expect(mapped.agentActivity).toHaveLength(256);
  expect(mapped.agentActivity[0]?.sequence).toBe(45);
  expect(mapped.agentActivity.at(-1)).toEqual({
    sequence: 300, observedAt: original.createdAt?.toISOString(),
    message: "Waiting for a model response", failed: true,
  });
});
