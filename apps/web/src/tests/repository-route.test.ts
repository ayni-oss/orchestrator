import { beforeEach, describe, expect, it, vi } from "vitest";

const bff = vi.hoisted(() => ({
  listRepositories: vi.fn(),
  saveRepository: vi.fn(),
}));
vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));
import { actions } from "../routes/repositories/+page.server.js";

describe("repository route actions", () => {
  const requestSignal = new AbortController().signal;
  beforeEach(() => vi.clearAllMocks());

  it("rejects invalid enrollment before invoking gRPC", async () => {
    const action = actions.enroll;
    if (action === undefined) throw new Error("enroll action is absent");
    const result = await action({
      locals: { requestId: "repo-invalid", requestSignal },
      request: new Request("http://127.0.0.1/repositories?/enroll", { method: "POST", body: new URLSearchParams({ id: "BAD ID" }) }),
    } as never);
    expect(result).toMatchObject({ status: 400, data: { error: { reason: "invalid_input" } } });
    expect(bff.saveRepository).not.toHaveBeenCalled();
  });

  it("sends a bounded enrollment through the BFF with a server idempotency key", async () => {
    bff.saveRepository.mockImplementationOnce(async (input) => ({ enrollment: input.repository }));
    const action = actions.enroll;
    if (action === undefined) throw new Error("enroll action is absent");
    const body = new URLSearchParams({ idempotencyKey: "00000000-0000-4000-8000-000000000099", id: "sandbox", owner: "ayni-oss", repository: "sandbox", installationId: "42", baseBranch: "main", lockSha256: "a".repeat(64) });
    const result = await action({ locals: { requestId: "repo-save", requestSignal }, request: new Request("http://127.0.0.1/repositories?/enroll", { method: "POST", body }) } as never);
    expect(result).toEqual({ action: "enroll", success: "Enrolled ayni-oss/sandbox." });
    expect(bff.saveRepository).toHaveBeenCalledWith(expect.objectContaining({ idempotencyKey: expect.stringMatching(/^[0-9a-f-]{36}$/u) }), expect.objectContaining({ requestId: "repo-save" }));
  });
});
