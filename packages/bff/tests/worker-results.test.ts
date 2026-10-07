import { describe, expect, it } from "vitest";
import { QualityResult, QualityOutcome, PullRequestReceipt } from "@ayni-orchestrator/grpc-bindings/server";
import { mapQuality, mapPullRequest } from "../src/mapping.js";

describe("worker quality and delivery", () => {
  it("preserves unavailable counts instead of inventing zero measurements", () => {
    const result = mapQuality(QualityResult.fromPartial({ outcome: QualityOutcome.QUALITY_OUTCOME_UNAVAILABLE }));
    expect(result.completedTargets).toBeUndefined();
    expect(result.failingOffenders).toBeUndefined();
    expect(result.outcome).toBe("unavailable");
  });
  it("requires well-formed bound evidence before displaying a pass", () => {
    const value = QualityResult.fromPartial({ outcome: QualityOutcome.QUALITY_OUTCOME_PASSED, evidence: { signalsDigest: "a".repeat(64), sourceFingerprint: "b".repeat(64), commit: "c".repeat(40) } });
    expect(mapQuality(value).outcome).toBe("passed");
    value.evidence!.signalsDigest = "malformed";
    expect(mapQuality(value).outcome).toBe("unavailable");
  });
  it("retains truthful failed-quality counts and bounds actionable findings", () => {
    const result = mapQuality(QualityResult.fromPartial({ outcome: QualityOutcome.QUALITY_OUTCOME_FAILED, completedTargets: 0, failingOffenders: 2, findings: Array.from({length:100}, () => "Repair\u0000the failing test") }));
    expect(result.outcome).toBe("failed");
    expect(result.completedTargets).toBe(0);
    expect(result.failingOffenders).toBe(2);
    expect(result.findings).toHaveLength(32);
    expect(result.findings[0]).toBe("Repair the failing test");
  });
  it("does not expose unsafe provider links", () => {
    expect(mapPullRequest(PullRequestReceipt.fromPartial({url:"javascript:alert(1)"})).url).toBeUndefined();
    expect(mapPullRequest(PullRequestReceipt.fromPartial({url:"https://github.com/example/repository/pull/3"})).url).toBe("https://github.com/example/repository/pull/3");
  });
});
