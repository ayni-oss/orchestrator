import {
  QualityOutcome,
  QualityResult,
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
import { mapDevelopmentAttempt, mapQuality } from "../src/mapping.js";
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
    registryConnectionId:"",imageNamespace:"", onboardingStatus:"pending", onboardingReason:"", onboardingRevision:"",
    status: RepositoryConnectionStatus.REPOSITORY_CONNECTION_STATUS_CONNECTED,
    createdAt: new Date("2026-01-01T00:00:00.000Z"),
    updatedAt: new Date("2026-01-01T00:00:01.000Z"),
    resourceVersion: "7",
  };
}

function profile(): RuntimeProfile {
  return {
    desiredImageRepository:"ghcr.io/team/sandbox",
    reconciliationMetadata: [], findings: [], preparationPhase: "", rebuildAvailable: false,
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
    snapshot: { limits:{codingTurns:24,qualityRepairLimit:3}, publicationBaseBranch:"main",
      repository: {
        repositoryConnectionId: "repository:1",
        repositorySnapshotId: "snapshot:1",
        revision: "a".repeat(40),
      },
      runtime: {
        runtimeProfileId: "profile:1",
        registryConnectionId:"connection:ghcr",registryRevision:"7",pullSecretName:"private-pull",
        runtimeProfileRevisionId: "profile-revision:1",
        profileKind: RuntimeProfileKind.RUNTIME_PROFILE_KIND_AYNI_MANAGED,
        runtimeArtifactId: "artifact:1",
        ociRepository: "ghcr.io/ayni-oss/sandbox",
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

  it("exposes runtime selection but keeps frozen Secret references out of browser DTOs",()=>{
    expect(mapRuntimeProfile({...profile(),registry:{registryConnectionId:"connection:ghcr"}}).registry).toEqual({registryConnectionId:"connection:ghcr"});
    expect(mapRuntimeProfile(profile()).registry).toBeUndefined();
    expect(mapRuntimeProfile(profile()).desiredImageRepository).toBe("ghcr.io/team/sandbox");
    expect(JSON.stringify(mapDevelopmentTask(task()))).not.toContain("private-pull");
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
      ociRepository: "ghcr.io/ayni-oss/sandbox",
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
      codingTurns: 17,
    })).toEqual(expect.objectContaining({
      runtimeProfileId: "profile:1",
      agent: { provider: "openai", agent: "coding-agent", model: "gpt-5" },
      deadline: { seconds: "60", nanos: 0 },
      codingTurns: 17,
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


describe("worker quality evidence", () => {
  it("preserves Ayni warning summaries and safely maps legacy or conflicting statuses", () => {
    const quality = QualityResult.fromPartial({rows:[
      {kind:"size",passed:true,status:"warn",summary:"max_lines=900 fail_count=0",target:"rust (workspace)"},
      {kind:"test",passed:true},
      {kind:"complexity",passed:false,status:"pass",summary:"max_cyclo=32"},
      {kind:"deps",passed:true,status:"https://untrusted.example/icon.svg"},
    ]});
    const rows = mapQuality(quality).rows;
    expect(rows[0]).toEqual({kind:"size",passed:true,status:"warn",summary:"max_lines=900 fail_count=0",target:"rust (workspace)"});
    expect(rows.map(row => row.status)).toEqual(["warn","pass","fail","pass"]);
    expect(rows[1]?.summary).toBe("");
  });
  it("preserves passing evidence with canonical sha256 prefixes and legacy hex digests", () => {
    for (const prefix of ["sha256:", ""]) {
      const evidence = {artifactPath:"evidence/signals.json",signalsDigest:prefix+"a".repeat(64),sourceFingerprint:prefix+"b".repeat(64),commit:"c".repeat(40)};
      const quality = QualityResult.fromPartial({outcome:QualityOutcome.QUALITY_OUTCOME_PASSED,evidence});
      expect(mapQuality(quality).outcome).toBe("passed");
      expect(mapQuality(quality).evidence).toEqual({signalsDigest:evidence.signalsDigest,sourceFingerprint:evidence.sourceFingerprint,commit:evidence.commit});
      for (const invalid of ["sha256:bad", "sha512:"+"a".repeat(64), "a".repeat(63)]) {
        quality.evidence = {...evidence,signalsDigest:invalid};
        expect(mapQuality(quality).outcome).toBe("unavailable");
        expect(mapQuality(quality).evidence).toBeUndefined();
      }
    }
  });
});
