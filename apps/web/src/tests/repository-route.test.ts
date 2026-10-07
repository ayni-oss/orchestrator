import { beforeEach, describe, expect, it, vi } from "vitest";

const bff = vi.hoisted(() => ({
  listRepositoryConnections: vi.fn(),
  getRuntimeProfile:vi.fn(),
  setRuntimeRegistry:vi.fn(),
  connectGithubRepository: vi.fn(),
  listGithubInstallations: vi.fn(),
  listGithubInstallationRepositories: vi.fn(),
}));
vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));
import { actions } from "../routes/repositories/+page.server.js";
import { actions as runtimeActions } from "../routes/repositories/[repositoryId]/+page.server.js";

describe("repository connection route actions", () => {
  const requestSignal = new AbortController().signal;
  beforeEach(() => {
    vi.clearAllMocks();
    bff.listGithubInstallations.mockResolvedValue([{ installationId: "42", owner: "ayni-oss" }]);
    bff.listGithubInstallationRepositories.mockResolvedValue([{ owner: "ayni-oss", name: "sandbox" }]);
  });

  it("rejects invalid provider identity before invoking gRPC", async () => {
    const action = actions.connect;
    if (action === undefined) throw new Error("connect action is absent");
    const result = await action({
      locals: { requestId: "repo-invalid", requestSignal },
      request: new Request("http://127.0.0.1/repositories?/connect", { method: "POST", body: new URLSearchParams({ repositoryId: "not-numeric" }) }),
    } as never);
    expect(result).toMatchObject({ status: 400, data: { error: { reason: "invalid_input" } } });
    expect(bff.connectGithubRepository).not.toHaveBeenCalled();
  });

  it("rejects unsupported or malformed explicit registry destinations before provider discovery", async () => {
    const action = actions.connect;
    if (action === undefined) throw new Error("connect action is absent");
    for (const destination of ["docker.io", "", "x".repeat(129)]) {
      const body = new URLSearchParams({
        idempotencyKey: "00000000-0000-4000-8000-000000000099",
        installationId: "42", owner: "ayni-oss", repository: "sandbox",
        registryDestination: destination,
      });
      const result = await action({ locals: { requestId: "registry-invalid", requestSignal }, request: new Request("http://127.0.0.1/repositories?/connect", { method: "POST", body }) } as never);
      expect(result).toMatchObject({ status: 400, data: { error: { reason: "unsupported_registry" } } });
    }
    expect(bff.listGithubInstallations).not.toHaveBeenCalled();
    expect(bff.connectGithubRepository).not.toHaveBeenCalled();
  });

  it("sends a provider-bounded connection with an idempotency key", async () => {
    bff.connectGithubRepository.mockImplementationOnce(async (input) => ({ id: "repository:1", displayName: `${input.owner}/${input.name}` }));
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
    expect(bff.connectGithubRepository).toHaveBeenCalledWith(expect.objectContaining({
      idempotencyKey: expect.stringMatching(/^[0-9a-f-]{36}$/u),
      owner:"ayni-oss", name:"sandbox",
    }), expect.objectContaining({ requestId: "repo-save" }));
  });
  it("runtime selection ignores browser-provided namespace and forwards only the connection",async()=>{
    bff.getRuntimeProfile.mockResolvedValue({id:"profile:1",repositoryConnectionId:"repository:1"});
    const body=new URLSearchParams({profileId:"profile:1",idempotencyKey:"00000000-0000-4000-8000-000000000099",resourceVersion:"2",registryConnectionId:"connection:ghcr",imageNamespace:"forged-namespace"});
    const result=await runtimeActions.registry!({params:{repositoryId:"repository:1"},locals:{requestId:"runtime-save",requestSignal},request:new Request("http://127.0.0.1/repositories/repository:1?/registry",{method:"POST",body})} as never);
    expect(result).toEqual({success:"Runtime image destination updated."});
    expect(bff.setRuntimeRegistry).toHaveBeenCalledWith({runtimeProfileId:"profile:1",idempotencyKey:"00000000-0000-4000-8000-000000000099",expectedResourceVersion:"2",registryConnectionId:"connection:ghcr"},expect.objectContaining({requestId:"runtime-save"}));
  });

});
