import { beforeEach, describe, expect, it, vi } from "vitest";

const bff = vi.hoisted(() => ({
  listRepositories: vi.fn(async () => ({ repositories: [], page: { nextPageToken: "" } })),
  listProviderModels: vi.fn(async () => ({ provider: "openai", models: [], observedAt: "2026-01-01T00:00:00.000Z", page: { nextPageToken: "" } })),
  listDevelopmentTasks: vi.fn(async () => ({ tasks: [], page: { nextPageToken: "" } })),
}));
vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));
import { load } from "../routes/+page.server.js";

describe("overview server boundary", () => {
  const requestSignal = new AbortController().signal;
  beforeEach(() => vi.clearAllMocks());

  it("loads operator data only through the server BFF and disables caching", async () => {
    const setHeaders = vi.fn();
    const result = await load({ locals: { requestId: "page-request", requestSignal }, request: new Request("http://127.0.0.1/"), setHeaders } as never);
    expect(result).toMatchObject({ backend: "connected", provider: "openai", requestId: "page-request" });
    expect(setHeaders).toHaveBeenCalledWith({ "cache-control": "no-store" });
    expect(bff.listRepositories).toHaveBeenCalledOnce();
    expect(bff.listProviderModels).toHaveBeenCalledOnce();
    expect(bff.listDevelopmentTasks).toHaveBeenCalledOnce();
  });

  it("renders a disconnected state without leaking an unexpected backend error", async () => {
    bff.listRepositories.mockRejectedValueOnce(new Error("private backend detail"));
    bff.listProviderModels.mockRejectedValueOnce(new Error("private backend detail"));
    bff.listDevelopmentTasks.mockRejectedValueOnce(new Error("private backend detail"));
    const result = await load({ locals: { requestId: "failed-request", requestSignal }, setHeaders: vi.fn() } as never);
    expect(result).toMatchObject({ backend: "disconnected", error: { reason: "internal", requestId: "failed-request" } });
    expect(JSON.stringify(result)).not.toContain("private backend detail");
  });
});
