import { beforeEach, describe, expect, it, vi } from "vitest";

interface StreamContext { requestId: string; signal?: AbortSignal }

const bff = vi.hoisted(() => ({
  watchDevelopmentTask: vi.fn(async (
    taskId: string,
    _cursor: string | undefined,
    context: StreamContext,
    onUpdate: (update: { kind: "development_task"; cursor: string; developmentTask: { id: string; resourceVersion: string; state: string } }) => void | Promise<void>,
  ) => {
    if (taskId.endsWith("0002")) {
      await new Promise<void>((resolve) => {
        if (context.signal?.aborted === true) resolve();
        else context.signal?.addEventListener("abort", () => resolve(), { once: true });
      });
      return;
    }
    for (let version = 8; version <= 41; version += 1) {
      onUpdate({
        kind: "development_task",
        cursor: String(version),
        developmentTask: {
          id: "00000000-0000-4000-8000-000000000001",
          resourceVersion: String(version),
          state: version === 8 ? "queued" : "running",
        },
      });
    }
  }),
}));

vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));
import { GET } from "../routes/_old/tasks/[taskId]/updates/+server.js";

const taskId = "00000000-0000-4000-8000-000000000001";
function locals(): { requestId: string; requestSignal: AbortSignal } {
  return { requestId: "stream-request", requestSignal: new AbortController().signal };
}

describe("same-origin task update relay", () => {
  beforeEach(() => {
    bff.watchDevelopmentTask.mockClear();
  });

  it("forwards the resume cursor and closes with a retryable error when the bounded buffer fills", async () => {
    const response = await GET({
      locals: locals(), params: { taskId },
      request: new Request(`http://127.0.0.1/_old/tasks/${taskId}/updates`, { headers: { "last-event-id": "7" } }),
    } as never);
    expect(response.headers.get("content-type")).toContain("text/event-stream");
    expect(response.headers.get("cache-control")).toBe("no-store");
    const body = await response.text();
    expect(body).toContain("id: 8\nevent: task\n");
    expect(body).toContain("id: 39\nevent: task\n");
    expect(body).not.toContain("id: 40\nevent: task\n");
    expect(body).toContain('"reason":"backpressure"');
    expect(bff.watchDevelopmentTask).toHaveBeenCalledWith(taskId, "7", expect.objectContaining({ requestId: "stream-request" }), expect.any(Function));
  });

  it.each(["not-a-version", "18446744073709551616"])(
    "rejects malformed or overflowing resume cursor %s before gRPC",
    async (cursor) => {
      const response = await GET({ locals: locals(), params: { taskId }, request: new Request(`http://127.0.0.1/_old/tasks/${taskId}/updates`, { headers: { "last-event-id": cursor } }) } as never);
      expect(response.status).toBe(400);
      expect(bff.watchDevelopmentTask).not.toHaveBeenCalled();
    },
  );

  it("rejects a malformed task identifier before gRPC", async () => {
    const response = await GET({ locals: locals(), params: { taskId: "------------------------------------" }, request: new Request("http://127.0.0.1/_old/tasks/invalid/updates") } as never);
    expect(response.status).toBe(400);
    expect(bff.watchDevelopmentTask).not.toHaveBeenCalled();
  });

  it("cancels the gRPC stream when the SSE reader disconnects", async () => {
    const streamingTaskId = "00000000-0000-4000-8000-000000000002";
    const response = await GET({ locals: locals(), params: { taskId: streamingTaskId }, request: new Request(`http://127.0.0.1/_old/tasks/${streamingTaskId}/updates`) } as never);
    await response.body?.cancel();
    const context = bff.watchDevelopmentTask.mock.calls[0]?.[2];
    expect(context?.signal?.aborted).toBe(true);
  });
});
