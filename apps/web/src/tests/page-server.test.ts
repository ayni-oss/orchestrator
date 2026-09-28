import { describe, expect, it, vi } from "vitest";

const bff = vi.hoisted(() => ({
  listRepositories: vi.fn(async () => ({ repositories: [], page: { nextPageToken: "" } })),
  listProviderModels: vi.fn(async () => ({
    provider: "openai",
    models: [],
    observedAt: "2026-01-01T00:00:00.000Z",
    page: { nextPageToken: "" },
  })),
  listDevelopmentTasks: vi.fn(async () => ({ tasks: [], page: { nextPageToken: "" } })),
  getDevelopmentTask: vi.fn(),
}));

vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));

import { actions, load } from "../routes/+page.server.js";

describe("server-only page boundary", () => {
  const requestSignal = new AbortController().signal;

  it("loads only through the BFF and disables response caching", async () => {
    const setHeaders = vi.fn();
    const result = await load({
      locals: { requestId: "page-request", requestSignal },
      request: new Request("http://127.0.0.1/"),
      setHeaders,
    } as never);
    expect(result).toMatchObject({ backend: "connected", provider: "openai", requestId: "page-request" });
    expect(setHeaders).toHaveBeenCalledWith({ "cache-control": "no-store" });
    expect(bff.listRepositories).toHaveBeenCalledOnce();
    expect(bff.listProviderModels).toHaveBeenCalledOnce();
    expect(bff.listDevelopmentTasks).toHaveBeenCalledOnce();
  });

  it("validates a form action before invoking gRPC", async () => {
    const action = actions.inspectTask;
    if (action === undefined) throw new Error("inspectTask action is absent");
    const result = await action({
      locals: { requestId: "action-request", requestSignal },
      request: new Request("http://127.0.0.1/?/inspectTask", {
        method: "POST",
        body: new URLSearchParams({ taskId: "------------------------------------" }),
      }),
    } as never);
    expect(result).toMatchObject({
      status: 400,
      data: { lookupError: { reason: "invalid_identifier", requestId: "action-request" } },
    });
    expect(bff.getDevelopmentTask).not.toHaveBeenCalled();
  });
});
