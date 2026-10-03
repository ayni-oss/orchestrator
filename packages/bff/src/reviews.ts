import {safeText,safeGithubLink} from "./onboarding.js";
import {
  type Assessment, type ChangeRequest, type EvidenceReference,
  assessmentConclusionToJSON, changeRequestStateToJSON,
} from "@ayni-orchestrator/grpc-bindings/server";


export function mapReview(value: ChangeRequest) {
  return {
    id: value.id, candidateId: value.candidateId || undefined,
    failure:value.failure ? {message:safeText(value.failure.message,512),remediation:safeText(value.failure.remediation,512)} : undefined,
    head: value.headRevision, state: changeRequestStateToJSON(value.state).replace("CHANGE_REQUEST_STATE_", "").toLowerCase(),
    url: safeGithubLink(value.provider?.provider?.$case === "github" ? value.provider.provider.github.htmlUrl : undefined),
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
    producer:value.producer ? `${safeText(value.producer.name,128)} ${safeText(value.producer.version,128)}` : undefined,
    workflowUrl: safeGithubLink(value.workflow?.provider?.$case === "github" ? value.workflow.provider.github.htmlUrl : undefined),
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
  return { id: value.id, assessmentId: value.assessmentId, name: safeText(value.name,128), digest: value.digest, sizeBytes: value.sizeBytes };
}
