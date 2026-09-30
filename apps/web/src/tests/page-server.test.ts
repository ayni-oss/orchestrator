import { beforeEach, describe, expect, it, vi } from "vitest";

const bff = vi.hoisted(() => ({
  listRepositoryConnections: vi.fn(async () => ({ repositoryConnections: [], page: { nextPageToken: "", snapshotCursor: "1" } })),
  listRuntimeProfiles: vi.fn(async () => ({ runtimeProfiles: [], page: { nextPageToken: "", snapshotCursor: "1" } })),
  listProviderModels: vi.fn(async () => ({ provider: "openai", models: [], observedAt: "2026-01-01T00:00:00.000Z", page: { nextPageToken: "", snapshotCursor: "1" } })),
  listDevelopmentTasks: vi.fn(async () => ({ developmentTasks: [], page: { nextPageToken: "", snapshotCursor: "1" } })),
}));
vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));
import { load } from "../routes/+page.server.js";

describe("overview server boundary", () => {
  const requestSignal = new AbortController().signal;
  beforeEach(() => vi.clearAllMocks());

  it("loads clean-v1 resources only through the server BFF and disables caching", async () => {
    const setHeaders = vi.fn();
    const result = await load({ locals: { requestId: "page-request", requestSignal }, request: new Request("http://127.0.0.1/"), setHeaders } as never);
    expect(result).toMatchObject({ backend: "connected", provider: "openai", requestId: "page-request" });
    expect(setHeaders).toHaveBeenCalledWith({ "cache-control": "no-store" });
    expect(bff.listRepositoryConnections).toHaveBeenCalledOnce();
    expect(bff.listRuntimeProfiles).toHaveBeenCalledOnce();
    expect(bff.listProviderModels).toHaveBeenCalledOnce();
    expect(bff.listDevelopmentTasks).toHaveBeenCalledOnce();
  });

  it("renders a disconnected state without leaking an unexpected backend error", async () => {
    for (const method of [bff.listRepositoryConnections, bff.listRuntimeProfiles, bff.listProviderModels, bff.listDevelopmentTasks]) {
      method.mockRejectedValueOnce(new Error("private backend detail"));
    }
    const result = await load({ locals: { requestId: "failed-request", requestSignal }, setHeaders: vi.fn() } as never);
    expect(result).toMatchObject({ backend: "disconnected", error: { reason: "internal", requestId: "failed-request" } });
    expect(JSON.stringify(result)).not.toContain("private backend detail");
  });
});
