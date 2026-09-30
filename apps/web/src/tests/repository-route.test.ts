import { beforeEach, describe, expect, it, vi } from "vitest";

const bff = vi.hoisted(() => ({
  listRepositoryConnections: vi.fn(),
  createRepositoryConnection: vi.fn(),
}));
vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));
import { actions } from "../routes/repositories/+page.server.js";

describe("repository connection route actions", () => {
  const requestSignal = new AbortController().signal;
  beforeEach(() => vi.clearAllMocks());

  it("rejects invalid provider identity before invoking gRPC", async () => {
    const action = actions.connect;
    if (action === undefined) throw new Error("connect action is absent");
    const result = await action({
      locals: { requestId: "repo-invalid", requestSignal },
      request: new Request("http://127.0.0.1/repositories?/connect", { method: "POST", body: new URLSearchParams({ repositoryId: "not-numeric" }) }),
    } as never);
    expect(result).toMatchObject({ status: 400, data: { error: { reason: "invalid_input" } } });
    expect(bff.createRepositoryConnection).not.toHaveBeenCalled();
  });

  it("sends a provider-bounded connection with an idempotency key", async () => {
    bff.createRepositoryConnection.mockImplementationOnce(async (input) => ({ id: "repository:1", displayName: input.displayName }));
    const action = actions.connect;
    if (action === undefined) throw new Error("connect action is absent");
    const body = new URLSearchParams({
      idempotencyKey: "00000000-0000-4000-8000-000000000099",
      repositoryId: "123",
      owner: "ayni-oss",
      repository: "sandbox",
      installationId: "42",
      defaultBranch: "main",
      displayName: "ayni-oss/sandbox",
    });
    const result = await action({ locals: { requestId: "repo-save", requestSignal }, request: new Request("http://127.0.0.1/repositories?/connect", { method: "POST", body }) } as never);
    expect(result).toEqual({ action: "connect", success: "Connected ayni-oss/sandbox." });
    expect(bff.createRepositoryConnection).toHaveBeenCalledWith(expect.objectContaining({
      idempotencyKey: expect.stringMatching(/^[0-9a-f-]{36}$/u),
      provider: expect.objectContaining({ kind: "github", repositoryId: "123" }),
    }), expect.objectContaining({ requestId: "repo-save" }));
  });
});
