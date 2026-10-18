import { beforeEach, describe, expect, it, vi } from "vitest";

const bff = vi.hoisted(() => ({
  listRepositoryConnections: vi.fn(),
  getRuntimeProfile:vi.fn(),
  getRepositoryConnection: vi.fn(),
  removeRepository: vi.fn(),
  analyzeRepository: vi.fn(),
  reconcileProfile: vi.fn(),
  setRuntimeRegistry:vi.fn(),
  connectGithubRepository: vi.fn(),
  listGithubInstallations: vi.fn(),
  listGithubInstallationRepositories: vi.fn(),
}));
vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));
import { actions } from "../routes/_old/repositories/+page.server.js";
import { actions as runtimeActions } from "../routes/_old/repositories/[repositoryId]/+page.server.js";

describe("repository connection route actions", () => {
  const requestSignal = new AbortController().signal;
  beforeEach(() => {
    vi.clearAllMocks();
    bff.getRepositoryConnection.mockResolvedValue({ id: "repository:1", displayName: "ayni-oss/sandbox", resourceVersion: "2" });
    bff.listGithubInstallations.mockResolvedValue([{ installationId: "42", owner: "ayni-oss" }]);
    bff.listGithubInstallationRepositories.mockResolvedValue([{ owner: "ayni-oss", name: "sandbox" }]);
  });

  it("rejects invalid provider identity before invoking gRPC", async () => {
    const action = actions.connect;
    if (action === undefined) throw new Error("connect action is absent");
    const result = await action({
      locals: { requestId: "repo-invalid", requestSignal },
      request: new Request("http://127.0.0.1/_old/repositories?/connect", { method: "POST", body: new URLSearchParams({ repositoryId: "not-numeric" }) }),
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
      const result = await action({ locals: { requestId: "registry-invalid", requestSignal }, request: new Request("http://127.0.0.1/_old/repositories?/connect", { method: "POST", body }) } as never);
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
    const result = await action({ locals: { requestId: "repo-save", requestSignal }, request: new Request("http://127.0.0.1/_old/repositories?/connect", { method: "POST", body }) } as never);
    expect(result).toEqual({ action: "connect", success: "Connected ayni-oss/sandbox." });
    expect(bff.connectGithubRepository).toHaveBeenCalledWith(expect.objectContaining({
      idempotencyKey: expect.stringMatching(/^[0-9a-f-]{36}$/u),
      owner:"ayni-oss", name:"sandbox",
    }), expect.objectContaining({ requestId: "repo-save" }));
  });
  it("runtime selection ignores browser-provided namespace and forwards only the connection",async()=>{
    bff.getRuntimeProfile.mockResolvedValue({id:"profile:1",repositoryConnectionId:"repository:1"});
    const body=new URLSearchParams({profileId:"profile:1",idempotencyKey:"00000000-0000-4000-8000-000000000099",resourceVersion:"2",registryConnectionId:"connection:ghcr",imageNamespace:"forged-namespace"});
    const result=await runtimeActions.registry!({params:{repositoryId:"repository:1"},locals:{requestId:"runtime-save",requestSignal},request:new Request("http://127.0.0.1/_old/repositories/repository:1?/registry",{method:"POST",body})} as never);
    expect(result).toEqual({success:"Runtime image destination updated."});
    expect(bff.setRuntimeRegistry).toHaveBeenCalledWith({runtimeProfileId:"profile:1",idempotencyKey:"00000000-0000-4000-8000-000000000099",expectedResourceVersion:"2",registryConnectionId:"connection:ghcr"},expect.objectContaining({requestId:"runtime-save"}));
  });


  it("Ayni opt-out is forwarded and ignores a submitted registry selection", async () => {
    bff.connectGithubRepository.mockResolvedValueOnce({ displayName: "ayni-oss/sandbox" });
    const body = new URLSearchParams({ idempotencyKey: "00000000-0000-4000-8000-000000000099", installationId: "42", owner: "ayni-oss", repository: "sandbox", useAyni: "false", registryConnectionId: "connection:ghcr" });
    await actions.connect!({ locals: { requestId: "ayni-opt-out", requestSignal }, request: new Request("http://localhost/repositories?/connect", { method: "POST", body }) } as never);
    expect(bff.connectGithubRepository).toHaveBeenCalledWith(expect.objectContaining({ useAyni: false, registryConnectionId: "" }), expect.anything());
  });

  it("Ayni opt-in forwards the selected registry connection", async () => {
    bff.connectGithubRepository.mockResolvedValueOnce({ displayName: "ayni-oss/sandbox" });
    const body = new URLSearchParams({ idempotencyKey: "00000000-0000-4000-8000-000000000099", installationId: "42", owner: "ayni-oss", repository: "sandbox", useAyni: "true", registryConnectionId: "connection:ghcr" });
    await actions.connect!({ locals: { requestId: "ayni-opt-in", requestSignal }, request: new Request("http://localhost/repositories?/connect", { method: "POST", body }) } as never);
    expect(bff.connectGithubRepository).toHaveBeenCalledWith(expect.objectContaining({ useAyni: true, registryConnectionId: "connection:ghcr" }), expect.anything());
  });
  it("analysis retries onboarding without authorizing runtime replacement", async () => {
    const body = new URLSearchParams({ idempotencyKey: "analysis-1", forceRebuild: "true" });
    const result = await runtimeActions.analyze!({ params: { repositoryId: "repository:1" }, locals: { requestId: "analysis", requestSignal }, request: new Request("http://127.0.0.1/_old/repositories/repository:1?/analyze", { method: "POST", body }) } as never);
    expect(result).toEqual({ success: "Repository analysis requested." });
    expect(bff.analyzeRepository).toHaveBeenCalledWith("repository:1", "analysis-1", expect.objectContaining({ requestId: "analysis" }));
    expect(bff.reconcileProfile).not.toHaveBeenCalled();
  });

});

describe("repository removal boundary", () => {
  function event(confirmation: string, resourceVersion = "2") {
    return {
      params: { repositoryId: "repository:1" },
      locals: { requestId: "remove-repository", requestSignal: new AbortController().signal },
      request: new Request("http://localhost/repositories/repository:1?/remove", {
        method: "POST", body: new URLSearchParams({ confirmation, resourceVersion, idempotencyKey: "00000000-0000-4000-8000-000000000099" }),
      }),
    } as never;
  }
  beforeEach(() => {
    vi.clearAllMocks();
    bff.getRepositoryConnection.mockResolvedValue({ id: "repository:1", displayName: "ayni-oss/sandbox", resourceVersion: "2" });
  });
  it("requires the server-observed repository name before removal", async () => {
    expect(await runtimeActions.remove!(event("another/repository"))).toMatchObject({ status: 400 });
    expect(bff.removeRepository).not.toHaveBeenCalled();
  });
  it("rejects an invalid version before invoking the backend", async () => {
    expect(await runtimeActions.remove!(event("ayni-oss/sandbox", "0"))).toMatchObject({ status: 400 });
    expect(bff.getRepositoryConnection).not.toHaveBeenCalled();
    expect(bff.removeRepository).not.toHaveBeenCalled();
  });
  it("forwards the observed version and redirects after confirmed removal", async () => {
    await expect(runtimeActions.remove!(event("ayni-oss/sandbox"))).rejects.toMatchObject({ status: 303, location: "/_old/repositories" });
    expect(bff.removeRepository).toHaveBeenCalledWith({ repositoryConnectionId: "repository:1", expectedResourceVersion: "2", idempotencyKey: "00000000-0000-4000-8000-000000000099" }, expect.objectContaining({ requestId: "remove-repository" }));
  });
});
