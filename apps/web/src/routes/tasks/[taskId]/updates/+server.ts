import { BffError, type BrowserErrorDto, type DevelopmentTaskDto } from "@ayni-orchestrator/bff/server";
import type { RequestHandler } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { isCanonicalIdentifier, isResourceVersion } from "$lib/server/validation.js";
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
  if (!isCanonicalIdentifier(params.taskId)) {
    return new Response("a valid task identifier is required", { status: 400 });
  }
  const cursor = request.headers.get("last-event-id") ?? undefined;
  if (cursor !== undefined && !isResourceVersion(cursor)) {
    return new Response("the task update cursor is invalid", { status: 400 });
  }
  const cancellation = new AbortController();
  const abort = (): void => cancellation.abort();
  locals.requestSignal.addEventListener("abort", abort, { once: true });

  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      const heartbeat = setInterval(() => {
        if ((controller.desiredSize ?? 1) > 0) controller.enqueue(encoder.encode(": keepalive\n\n"));
      }, 15_000);
      const finish = (): void => {
        clearInterval(heartbeat);
        locals.requestSignal.removeEventListener("abort", abort);
        try {
          controller.close();
        } catch {
          // The browser already closed the stream.
        }
      };
      void getBff().watchDevelopmentTask(
        params.taskId,
        cursor,
        { requestId: locals.requestId, signal: cancellation.signal },
        (task: DevelopmentTaskDto) => {
          if ((controller.desiredSize ?? 1) <= 0) {
            throw new BffError({
              status: 503,
              reason: "backpressure",
              message: "the task update consumer is too slow",
              requestId: locals.requestId,
              retryable: true,
            });
          }
          controller.enqueue(encoder.encode(
            `id: ${task.resourceVersion}\nevent: task\ndata: ${JSON.stringify(task)}\n\n`,
          ));
        },
      ).catch((error: unknown) => {
        if (!cancellation.signal.aborted && (controller.desiredSize ?? 1) > 0) {
          controller.enqueue(encoder.encode(`event: error\ndata: ${JSON.stringify(errorDto(error, locals.requestId))}\n\n`));
        }
      }).finally(finish);
    },
    cancel() {
      cancellation.abort();
      request.signal.removeEventListener("abort", abort);
    },
  });

  return new Response(body, {
    headers: {
      "cache-control": "no-store",
      "content-type": "text/event-stream; charset=utf-8",
      "x-accel-buffering": "no",
    },
  });
};
