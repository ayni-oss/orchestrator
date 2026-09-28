import type { Handle } from "@sveltejs/kit";
import { acceptedRequestId } from "$lib/server/request-id.js";
import { browserRequestSignal } from "$lib/server/request-signal.js";

export const handle: Handle = async ({ event, resolve }) => {
  event.locals.requestId = acceptedRequestId(event.request.headers.get("x-request-id"));
  event.locals.requestSignal = browserRequestSignal(event.request);
  const response = await resolve(event);
  const headers = new Headers(response.headers);
  headers.set("x-request-id", event.locals.requestId);
  headers.set("x-content-type-options", "nosniff");
  headers.set("referrer-policy", "no-referrer");
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
};
