import { enhance as kitEnhance } from "$app/forms";
import type { SubmitFunction } from "@sveltejs/kit";
import { notify, redirectNotification } from "$lib/notifications.js";

/** Preserve Kit form semantics and notify successful actions that navigate away. */
export function enhance(form: HTMLFormElement, submit?: SubmitFunction) {
  return kitEnhance<Record<string, unknown>, Record<string, unknown>>(form, async input => {
    const complete = await submit?.(input);
    return async event => {
      if((event.result.type==="success"||event.result.type==="failure")&&event.result.data)event.result.data.notificationId=crypto.randomUUID();
      if (event.result.type === "error") {
        notify("error", "Action failed", event.result.error.message || "The request could not be completed. Try again.");
        // Keep the editor and its fields available; Kit would otherwise navigate
        // to the full-page error boundary. Still run cleanup in custom callbacks.
        if (complete) await complete({ ...event, update: async () => {} });
        return;
      }
      if (complete) await complete(event);
      else await event.update();
      redirectNotification(event.result, input.action);
    };
  });
}
