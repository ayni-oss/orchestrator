import { BffError, type BrowserErrorDto, type DevelopmentTaskUpdateDto } from "@ayni-orchestrator/bff/server";
import type { RequestHandler } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { isResourceVersion } from "$lib/server/validation.js";

const RESOURCE_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;

const encoder = new TextEncoder();

function errorDto(error: unknown, requestId: string): BrowserErrorDto {
  if (error instanceof BffError) return error.toDto();
  return {
    status: 500,
    reason: "internal",
    message: "the task update stream ended unexpectedly",
    requestId,
    retryable: false,
  };
}

export const GET: RequestHandler = ({ locals, params, request }) => {
  if (!RESOURCE_ID.test(params.taskId)) {
    return new Response("a valid task identifier is required", { status: 400 });
  }
  const cursor = request.headers.get("last-event-id") ?? undefined;
  if (cursor !== undefined && !isResourceVersion(cursor)) {
    return new Response("the task update cursor is invalid", { status: 400 });
  }

  const cancellation = new AbortController();
  const abort = (): void => cancellation.abort();
  if (locals.requestSignal.aborted) cancellation.abort();
  else locals.requestSignal.addEventListener("abort", abort, { once: true });
  let closed = false;
  let heartbeat: ReturnType<typeof setInterval> | undefined;

  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      const finish = (): void => {
        if (closed) return;
        closed = true;
        if (heartbeat !== undefined) clearInterval(heartbeat);
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
            message: "the task update consumer is too slow",
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
      heartbeat = setInterval(() => {
        enqueue("event: heartbeat\ndata: {}\n\n");
      }, 15_000);

      void (async () => {
        try {
          await getBff().watchDevelopmentTask(
            params.taskId,
            cursor,
            { requestId: locals.requestId, signal: cancellation.signal },
            (update: DevelopmentTaskUpdateDto) => {
              if (update.kind === "development_task") {
                enqueue(`id: ${update.cursor}\nevent: task\ndata: ${JSON.stringify(update.developmentTask)}\n\n`);
              } else {
                enqueue(`id: ${update.cursor}\nevent: related\ndata: ${JSON.stringify(update)}\n\n`);
              }
            },
          );
        } catch (error) {
          if (!cancellation.signal.aborted) enqueue(`event: error\ndata: ${JSON.stringify(errorDto(error, locals.requestId))}\n\n`);
        } finally {
          finish();
        }
      })();
    },
    cancel() {
      closed = true;
      if (heartbeat !== undefined) clearInterval(heartbeat);
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
