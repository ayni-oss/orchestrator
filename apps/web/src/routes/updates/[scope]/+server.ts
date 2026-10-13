import { BffError, type BrowserErrorDto } from "@ayni-orchestrator/bff/server";
import type { RequestHandler } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { isResourceVersion } from "$lib/server/validation.js";


const encoder = new TextEncoder();

function errorDto(error: unknown, requestId: string): BrowserErrorDto {
  if (error instanceof BffError) return error.toDto();
  return {
    status: 500,
    reason: "internal",
    message: "the resource update stream ended unexpectedly",
    requestId,
    retryable: false,
  };
}

export const GET: RequestHandler = ({ locals, params, request }) => {
  const scope = params.scope;
  if (scope !== "connections" && scope !== "certificates" && scope !== "projects" && scope !== "runtimes" && scope !== "repositories") return new Response("invalid resource scope", { status: 404 });
  const cursor = request.headers.get("last-event-id") ?? undefined;
  if (cursor !== undefined && cursor !== "0" && !isResourceVersion(cursor)) {
    return new Response("the resource update cursor is invalid", { status: 400 });
  }

  const cancellation = new AbortController();
  const abort = (): void => cancellation.abort();
  if (locals.requestSignal.aborted) cancellation.abort();
  else locals.requestSignal.addEventListener("abort", abort, { once: true });
  let closed = false;
  let changedTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingCursor: string | undefined;
  let heartbeat: ReturnType<typeof setInterval> | undefined;

  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      const finish = (): void => {
        if (closed) return;
        closed = true;
        if (heartbeat !== undefined) clearInterval(heartbeat);
        clearTimeout(changedTimer);
        locals.requestSignal.removeEventListener("abort", abort);
        try {
          controller.close();
        } catch {
          // The browser already closed the stream.
        }
      };
      const enqueue = (payload: string): boolean => {
        if (closed) return false;
        if ((controller.desiredSize ?? 1) <= 0) {
          const overflow = new BffError({
            status: 503,
            reason: "backpressure",
            message: "the resource update consumer is too slow",
            requestId: locals.requestId,
            retryable: true,
          });
          controller.enqueue(encoder.encode(`event: error\ndata: ${JSON.stringify(overflow.toDto())}\n\n`));
          cancellation.abort();
          finish();
          return false;
        }
        controller.enqueue(encoder.encode(payload));
        return true;
      };
      const flush = (): void => {
        changedTimer = undefined;
        if (pendingCursor === undefined) return;
        const nextCursor = pendingCursor;
        pendingCursor = undefined;
        enqueue(`id: ${nextCursor}\nevent: changed\ndata: {}\n\n`);
      };
      heartbeat = setInterval(() => {
        enqueue("event: heartbeat\ndata: {}\n\n");
      }, 15_000);

      void (async () => {
        try {
          await getBff().watchResources(scope, cursor,
            { requestId: locals.requestId, signal: cancellation.signal },
            (nextCursor: string) => {
              pendingCursor = nextCursor;
              changedTimer ??= setTimeout(flush, 30);
            },
          );
        } catch (error) {
          if (!cancellation.signal.aborted) enqueue(`event: error\ndata: ${JSON.stringify(errorDto(error, locals.requestId))}\n\n`);
        } finally {
          flush();
          finish();
        }
      })();
    },
    cancel() {
      closed = true;
      if (heartbeat !== undefined) clearInterval(heartbeat);
      clearTimeout(changedTimer);
      cancellation.abort();
      locals.requestSignal.removeEventListener("abort", abort);
    },
  }, { highWaterMark: 32, size: () => 1 });

  return new Response(body, {
    headers: {
      "cache-control": "no-store",
      "content-type": "text/event-stream; charset=utf-8",
      "x-accel-buffering": "no",
    },
  });
};
