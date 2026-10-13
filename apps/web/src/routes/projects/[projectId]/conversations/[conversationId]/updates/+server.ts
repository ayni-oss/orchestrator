import { BffError, type BrowserErrorDto } from "@ayni-orchestrator/bff/server";
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
    message: "the conversation update stream ended unexpectedly",
    requestId,
    retryable: false,
  };
}

export const GET: RequestHandler = async ({ locals, params, request, url }) => {
  if (!RESOURCE_ID.test(params.conversationId)) {
    return new Response("a valid conversation identifier is required", { status: 400 });
  }
  const cursor = request.headers.get("last-event-id") ?? url.searchParams.get("after") ?? undefined;
  if (cursor !== undefined && !isResourceVersion(cursor)) {
    return new Response("the conversation update cursor is invalid", { status: 400 });
  }

  try {
    const conversation=await getBff().getProjectConversation(params.conversationId,{requestId:locals.requestId,signal:locals.requestSignal});
    if(conversation.projectId!==params.projectId)return new Response("Conversation does not belong to this project",{status:404});
  }catch(error){const mapped=errorDto(error,locals.requestId);return Response.json({error:mapped},{status:mapped.status});}
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
            message: "the conversation update consumer is too slow",
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
      enqueue("event: heartbeat\ndata: {}\n\n");
      heartbeat = setInterval(() => {
        enqueue("event: heartbeat\ndata: {}\n\n");
      }, 15_000);

      void (async () => {
        try {
          await getBff().watchProjectConversation(
            params.conversationId,
            cursor,
            { requestId: locals.requestId, signal: cancellation.signal },
            (updateCursor:string) => {
              enqueue(`id: ${updateCursor}\nevent: changed\ndata: {}\n\n`);
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
