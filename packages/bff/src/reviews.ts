import {
  type Assessment, type ChangeRequest, type EvidenceReference,
  assessmentConclusionToJSON, changeRequestStateToJSON,
} from "@ayni-orchestrator/grpc-bindings/server";

function githubLink(value: string | undefined): string | undefined {
  if (!value || value.length > 2048) return undefined;
  try {
    const url = new URL(value);
    return url.protocol === "https:" && url.hostname === "github.com" && !url.username && !url.password ? url.href : undefined;
  } catch { return undefined; }
}

export function mapReview(value: ChangeRequest) {
  return {
    id: value.id, candidateId: value.candidateId || undefined,
    head: value.headRevision, state: changeRequestStateToJSON(value.state).replace("CHANGE_REQUEST_STATE_", "").toLowerCase(),
    url: githubLink(value.provider?.provider?.$case === "github" ? value.provider.provider.github.htmlUrl : undefined),
    readiness: value.readiness ? {
      policy: value.readiness.policy, state: value.readiness.state,
      source: value.readiness.source, authorizedHead: value.readiness.authorizedHead,
      assessmentId: value.readiness.assessmentId, diagnostic: value.readiness.diagnostic.slice(0, 256),
    } : undefined,
  };
}

export function mapAssessment(value: Assessment) {
  const managed = value.details?.$case === "ayniManaged" ? value.details.ayniManaged : undefined;
  return {
    id: value.id, head: value.headRevision,
    conclusion: assessmentConclusionToJSON(value.conclusion).replace("ASSESSMENT_CONCLUSION_", "").toLowerCase(),
    profileRevision: value.runtimeProfileRevisionId,
    workflowUrl: githubLink(value.workflow?.provider?.$case === "github" ? value.workflow.provider.github.htmlUrl : undefined),
    createdAt: value.createdAt?.toISOString(),
    evidenceIds: value.evidenceReferenceIds.slice(0, 32),
    managed: managed ? {
      manifestDigest: managed.manifestDigest,
      runtimeDigest: managed.runtimeDigest,
      completed: managed.completedTargetCount, skipped: managed.skippedTargetCount,
      warnings: managed.warningCount, failures: managed.failureCount,
    } : undefined,
  };
}

export function mapEvidence(value: EvidenceReference) {
  return { id: value.id, assessmentId: value.assessmentId, name: value.name, digest: value.digest, sizeBytes: value.sizeBytes };
}
