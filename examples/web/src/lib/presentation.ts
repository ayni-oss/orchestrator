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

export function formatDate(value: string): string {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return "Unknown time";
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(date);
}

export function formatDuration(seconds: number | undefined): string {
  if (seconds === undefined) return "—";
  if (seconds < 60) return `${seconds.toFixed(seconds < 10 ? 1 : 0)}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${Math.round(seconds % 60)}s`;
}

export function formatTokens(value: string | undefined): string {
  if (value === undefined) return "—";
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? new Intl.NumberFormat().format(parsed) : value;
}

export function formatCost(micros: string | undefined, currency = "USD"): string {
  if (micros === undefined) return "Unavailable";
  const parsed = Number(micros);
  if (!Number.isFinite(parsed)) return `${micros} µ${currency}`;
  return new Intl.NumberFormat(undefined, { style: "currency", currency, minimumFractionDigits: 4, maximumFractionDigits: 6 }).format(parsed / 1_000_000);
}
