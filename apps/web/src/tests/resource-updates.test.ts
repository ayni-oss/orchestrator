import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const bff = vi.hoisted(() => ({ watchResources: vi.fn() }));
vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));
import { GET } from "../routes/updates/[scope]/+server.js";

function request(scope = "all", cursor?: string) {
  return GET({
    locals: { requestId: "resource-stream", requestSignal: new AbortController().signal },
    params: { scope },
    request: new Request(`http://localhost/updates/${scope}`, { headers: cursor === undefined ? {} : { "last-event-id": cursor } }),
  } as never);
}

describe("combined resource update relay", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    bff.watchResources.mockReset().mockImplementation((_scope, _cursor, context) => new Promise<void>(resolve => {
      context.signal.addEventListener("abort", () => resolve(), { once: true });
    }));
  });
  afterEach(() => vi.useRealTimers());

  it("multiplexes five scoped watches with bounded coalescing and cancels all on disconnect", async () => {
    const response = await request();
    expect(bff.watchResources.mock.calls.map(call => call[0])).toEqual(["connections", "certificates", "projects", "runtimes", "repositories"]);
    expect(bff.watchResources.mock.calls.every(call => call[1] === undefined)).toBe(true);
    const reader = response.body!.getReader();
    const first = reader.read();
    bff.watchResources.mock.calls[0]![3]("7");
    bff.watchResources.mock.calls[0]![3]("8");
    bff.watchResources.mock.calls[1]![3]("9");
    await vi.advanceTimersByTimeAsync(30);
    const decoder = new TextDecoder();
    expect(decoder.decode((await first).value)).toBe('event: changed\ndata: {"scope":"connections"}\n\n');
    expect(decoder.decode((await reader.read()).value)).toBe('event: changed\ndata: {"scope":"certificates"}\n\n');
    await reader.cancel();
    expect(bff.watchResources.mock.calls.every(call => call[2].signal.aborted)).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("rejects a shared cursor rather than applying it to independent streams", async () => {
    expect((await request("all", "8")).status).toBe(400);
    expect(bff.watchResources).not.toHaveBeenCalled();
  });

  it("preserves scoped cursor replay for a dedicated subscription", async () => {
    const response = await request("projects", "8");
    expect(bff.watchResources).toHaveBeenCalledWith("projects", "8", expect.any(Object), expect.any(Function));
    const reader = response.body!.getReader();
    const next = reader.read();
    bff.watchResources.mock.calls[0]![3]("9");
    await vi.advanceTimersByTimeAsync(30);
    expect(new TextDecoder().decode((await next).value)).toBe("id: 9\nevent: changed\ndata: {}\n\n");
    await reader.cancel();
  });
});
