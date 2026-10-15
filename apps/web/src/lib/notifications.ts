import type { ActionResult } from "@sveltejs/kit";
import { toast } from "svelte-sonner";

export type NotificationKind = "success" | "error" | "warning" | "info";

/** Shared presentation for transient events. Never pass credentials or raw responses. */
export function notify(kind: NotificationKind, title: string, message?: string, requestId?: string) {
  const description = [message, requestId ? `Request ${requestId}` : undefined].filter(Boolean).join("\n");
  return toast[kind](title, {
    ...(description ? { description } : {}),
    // Failures remain available until explicitly dismissed.
    duration: kind === "error" ? Infinity : 6000,
  });
}

export function actionNotification(result: Record<string, unknown> | null | undefined) {
  const error = result?.error as { message?: string; requestId?: string } | undefined;
  if (error?.message) notify("error", "Action failed", error.message, error.requestId);
  else if (typeof result?.success === "string") notify("success", result.success);
  else {
    // Superforms uses form.message for server failures; field errors stay inline.
    const form = result?.form as { message?: unknown } | undefined;
    if (typeof form?.message === "string") notify("error", "Action failed", form.message);
  }
}

/** Per-layout event history: repeated saves notify again, unchanged polling failures do not. */
export function createNotificationEvents() {
  let lastForm: unknown;
  let lastActionId: string|undefined;
  let route = "";
  let previousFailures = new Set<string>();
  return {
    action(result: Record<string, unknown> | null | undefined) {
      const actionId=typeof result?.notificationId==="string"?result.notificationId:undefined;
      if (result === lastForm || (actionId!==undefined&&actionId===lastActionId)) return;
      if(actionId!==undefined)lastActionId=actionId;
      lastForm = result;
      actionNotification(result);
    },
    load(pathname: string, data: Record<string, unknown>) {
      if (route !== pathname) { route = pathname; previousFailures = new Set(); }
      const failures = new Set<string>();
      for (const [key, value] of Object.entries(data)) {
        if (key !== "error" && !key.endsWith("Error")) continue;
        if (!value || typeof value !== "object" || !("message" in value) || typeof value.message !== "string") continue;
        const signature = `${key}:${value.message}`;
        failures.add(signature);
        if (!previousFailures.has(signature)) {
          const requestId = "requestId" in value && typeof value.requestId === "string" ? value.requestId : undefined;
          notify("error", "Data unavailable", value.message, requestId);
        }
      }
      previousFailures = failures;
    },
  };
}


/** Redirects do not populate page.form; only confirmed mutation redirects notify. */
export function redirectNotification(result: ActionResult, action: URL) {
  if (result.type !== "redirect" || result.status !== 303) return;
  const operation = [...action.searchParams.keys()].find(key => key.startsWith("/"))?.slice(1);
  const messages: Record<string, string> = {
    create: action.pathname === "/workspaces" ? "Workspace created." : action.pathname === "/projects" ? "Project created." : "Task submitted.",
    createTask: "Task submitted.",
    remove: "Repository removed.",
    cancel: "Task cancellation requested.",
    retry: "Task retry requested.",
    retryPublication: "Publication retry requested.",
  };
  if (operation && messages[operation]) notify("success", messages[operation]);
}
