import type { RuntimeProfileRevision, AutomationInstallation, RepositorySnapshot, InspectAyniRepositoryResponse, Candidate, DevelopmentAttempt } from "@ayni-orchestrator/grpc-bindings/server";
import { automationInstallationStateToJSON, candidateStateToJSON } from "@ayni-orchestrator/grpc-bindings/server";

export function safeText(value: string | undefined, max = 256): string {
  return (value ?? "").replace(/[\u0000-\u001f\u007f]/gu, " ").slice(0, max);
}
export function safeGithubLink(value: string | undefined): string | undefined {
  if (!value || value.length > 2048) return undefined;
  try { const u = new URL(value); return u.protocol === "https:" && u.hostname === "github.com" && !u.username && !u.password && !u.port ? u.href : undefined; } catch { return undefined; }
}
export function mapRevision(v: RuntimeProfileRevision) {
  const c = v.configuration;
  return { id: v.id, repositoryRevision: v.repositoryRevision, snapshotId: v.repositorySnapshotId,
    managed: c?.$case === "ayniManaged" ? { fingerprint: c.ayniManaged.lockFingerprint, ayniVersion: safeText(c.ayniManaged.ayniVersion, 128), template: safeText(c.ayniManaged.workflowTemplateVersion,128), repository: safeText(c.ayniManaged.ociRepository,512) } : undefined };
}
export function mapAutomation(v: AutomationInstallation) {
  return { id:v.id, version:safeText(v.version,128), state:automationInstallationStateToJSON(v.state).replace("AUTOMATION_INSTALLATION_STATE_", "").toLowerCase(),
    pullNumber:v.provider?.$case === "github" ? v.provider.github.setupPullRequestNumber : undefined,
    reason: v.failure ? safeText(v.failure.message) : undefined };
}
export function mapSnapshot(v: RepositorySnapshot) { return { id:v.id, revision:v.revision, defaultBranch:safeText(v.defaultBranch) }; }
export function mapInspection(v: InspectAyniRepositoryResponse) {
  return { snapshotId:v.repositorySnapshotId, head:v.repositoryRevision, fingerprint:v.lockFingerprint, version:safeText(v.ayniVersion,128), template:safeText(v.workflowTemplateVersion,128), repository:safeText(v.ociRepository,512), policyPresent:v.policyPresent, lockCompatible:v.lockCompatible, trustPinned:v.trustPolicyPinned, reason:safeText(v.reason), workflows:v.workflows.slice(0,3).map(w=>({path:safeText(w.path,256),contents:w.contents.slice(0,16000)})) };
}
export function mapCandidate(v: Candidate) {
  const identity=v.identity?.identity;
  return { id:v.id, state:candidateStateToJSON(v.state).replace("CANDIDATE_STATE_", "").toLowerCase(), head:identity?.$case === "git" ? identity.git.commitSha : undefined,
    advisory:v.codingAdvisory ? safeText(v.codingAdvisory,512) : undefined };
}
export function mapAttemptUsage(v: DevelopmentAttempt) {
  const t=v.modelUsage?.total;
  return { input:t?.inputTokens, output:t?.outputTokens, total:t?.totalTokens, interactions:t?.interactionCount ?? 0 };
}

const metadataKeys = new Set(["default-sha", "fingerprint", "resolved-digest", "discovery-reference", "reason-code", "source_kind", "source_revision", "lock_fingerprint", "runtime_digest", "last_success", "next_retry", "reason", "platform", "source-kind", "source-revision", "lock-fingerprint", "runtime-digest", "last-success", "next-retry"]);
export function safeReconciliation(values: Array<{key:string;value:string}>) {
  return values.filter(v=>metadataKeys.has(v.key)).slice(0,16).map(v=>({key:v.key,value:safeText(v.value,512)}));
}
