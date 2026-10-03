import { describe, expect, it } from "vitest";
import { Assessment, ChangeRequest, EvidenceReference } from "@ayni-orchestrator/grpc-bindings/server";
import { mapAssessment, mapEvidence, mapReview } from "../src/reviews.js";

describe("bounded review projections", () => {
  it("does not expose untrusted links or raw evidence metadata", () => {
    const review = mapReview(ChangeRequest.fromPartial({ provider: { provider: { $case: "github", github: { htmlUrl: "javascript:alert(1)" } } } }));
    expect(review.url).toBeUndefined();
    const evidence = mapEvidence(EvidenceReference.fromPartial({ name: "signals.json", digest: "sha256:abc", metadata: [{ key: "log", value: "credential-bearing log" }] }));
    expect(JSON.stringify(evidence)).not.toContain("credential-bearing");
    expect(evidence.name).toBe("signals.json");
  });

  it("keeps exact-head history and readiness source independent", () => {
    const head = "a".repeat(40);
    const review = mapReview(ChangeRequest.fromPartial({ headRevision: head, readiness: { policy: "DRAFT_ONLY", state: "manual", source: "maintainer" } }));
    expect(review.readiness?.source).toBe("maintainer");
    expect(review.head).toBe(head);
    const assessment = mapAssessment(Assessment.fromPartial({ headRevision: "b".repeat(40), evidenceReferenceIds: Array.from({ length: 40 }, (_, i) => `evidence:${i}`) }));
    expect(assessment.head).not.toBe(review.head);
    expect(assessment.evidenceIds).toHaveLength(32);
  });
});
