export type StatusTone = "neutral" | "active" | "success" | "warning" | "danger";

export function words(value: string): string {
  return value.replaceAll("_", " ").replace(/^./u, (letter) => letter.toUpperCase());
}

export function statusTone(status: string): StatusTone {
  if (["connected", "ready", "succeeded", "verified", "installed", "merged"].includes(status)) return "success";
  if (["failed", "degraded", "revoked", "deadline_exceeded", "unsupported", "rejected", "unavailable", "quality_failed", "policy_rejected", "infrastructure_failed"].includes(status)) return "danger";
  if (["setup_required", "setup_pr_open", "awaiting_environment", "waiting_for_runtime", "cancelled", "incomplete", "stale"].includes(status)) return "warning";
  if (["connecting", "updating", "queued", "running", "publishing", "discovered", "verifying"].includes(status)) return "active";
  return "neutral";
}
