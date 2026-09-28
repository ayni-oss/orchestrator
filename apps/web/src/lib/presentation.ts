export type StatusTone = "neutral" | "active" | "success" | "warning" | "danger";

export function words(value: string): string {
  return value.replaceAll("_", " ").replace(/^./u, (letter) => letter.toUpperCase());
}

export function phaseTone(phase: string): StatusTone {
  if (phase === "completed") return "success";
  if (["failed", "unsupported"].includes(phase)) return "danger";
  if (phase === "waiting_for_input") return "warning";
  if (["coding", "validation_queued", "validating", "pr_queued", "preparing_pr"].includes(phase)) return "active";
  return "neutral";
}

export function preparationTone(status: string): StatusTone {
  if (status === "ready") return "success";
  if (status === "failed") return "danger";
  if (status === "setup_required") return "warning";
  if (status === "preparing") return "active";
  return "neutral";
}

export function stageLabel(stage: string): string {
  return stage === "pr_preparation" ? "Pull request" : words(stage);
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
