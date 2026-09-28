import {
  DevelopmentTaskPhase,
  PreparationStatus,
  type DevelopmentTask,
} from "@ayni-orchestrator/grpc-bindings/server";
import { describe, expect, it } from "vitest";
import {
  BffMappingError,
  mapDevelopmentTask,
  mapPreparationStatus,
  mapTaskPhase,
  toProtoTaskInput,
} from "../src/index.js";

function task(): DevelopmentTask {
  return {
    id: "task-id",
    input: {
      repositoryId: "sandbox",
      model: "gpt-5",
      startRevision: "a".repeat(40),
      request: "Correct the formula",
      acceptanceCriteria: ["three becomes eight"],
      changeConstraints: { protectedPaths: ["docs"], allowTestEdits: false },
      deadline: { seconds: "300", nanos: 0 },
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
    resourceVersion: "7",
  };
}

describe("browser DTO mappings", () => {
  it("maps allowlisted task fields without returning transport objects", () => {
    const mapped = mapDevelopmentTask(task());
    expect(mapped).toEqual(expect.objectContaining({
      id: "task-id",
      phase: "queued",
      resourceVersion: "7",
      input: expect.objectContaining({ deadlineSeconds: 300 }),
    }));
    expect(mapped).not.toHaveProperty("$type");
    expect(JSON.parse(JSON.stringify(mapped))).toEqual(mapped);
  });

  it("handles additive enum values as unknown", () => {
    expect(mapTaskPhase(999 as DevelopmentTaskPhase)).toBe("unknown");
    expect(mapPreparationStatus(999 as PreparationStatus)).toBe("unknown");
  });

  it("rejects incomplete required transport values", () => {
    expect(() => mapDevelopmentTask({ ...task(), repository: undefined })).toThrow(BffMappingError);
    expect(() => mapDevelopmentTask({ ...task(), modelUsage: undefined })).toThrow(BffMappingError);
    const missingConstraints = task();
    if (missingConstraints.input !== undefined) missingConstraints.input.changeConstraints = undefined;
    expect(() => mapDevelopmentTask(missingConstraints)).toThrow(BffMappingError);
    expect(() => mapDevelopmentTask({ ...task(), draftPrUrl: "javascript:alert(1)" })).toThrow(BffMappingError);
  });

  it("builds explicit transport input from browser input", () => {
    expect(toProtoTaskInput({
      repositoryId: "sandbox",
      model: "gpt-5",
      startRevision: "a".repeat(40),
      request: "Correct the formula",
      acceptanceCriteria: ["three becomes eight"],
      changeConstraints: { protectedPaths: ["docs"], allowTestEdits: false },
      deadlineSeconds: 60,
    })).toEqual(expect.objectContaining({ deadline: { seconds: "60", nanos: 0 } }));
  });
});
