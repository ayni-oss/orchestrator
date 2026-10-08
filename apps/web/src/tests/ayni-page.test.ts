import { beforeEach, expect, it, vi } from "vitest";
const bff = vi.hoisted(() => ({ getBuildService: vi.fn(), listAyniReleases: vi.fn(), listConnections: vi.fn(), listCertificates: vi.fn(), updateBuildService: vi.fn(), testBuildService: vi.fn(), startBuilderInstallation: vi.fn(), cancelBuilderInstallation: vi.fn() }));
vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));
import { load, actions } from "../routes/ayni/+page.server.js";
const locals = { requestId: "request", requestSignal: new AbortController().signal };
beforeEach(() => {
  vi.clearAllMocks();
  bff.getBuildService.mockResolvedValue({ status: "configured" });
  bff.listAyniReleases.mockResolvedValue({ releases: [], discoveryStatus: "unavailable" });
  bff.listConnections.mockResolvedValue([{ id: "ready", kind: "ghcr", enabled: true, status: "ready" }, { id: "failed", kind: "ghcr", enabled: true, status: "failed" }]);
  bff.listCertificates.mockResolvedValue([{ id: "ready", enabled: true, status: "ready" }, { id: "disabled", enabled: false, status: "ready" }]);
});
it("keeps configuration available during release discovery outages and filters test authority", async () => {
  const setHeaders = vi.fn();
  const result = await load({ locals, setHeaders, url: new URL("http://localhost/ayni?page=999") } as never);
  expect(result).toMatchObject({ settings: { status: "configured" }, registries: [{ id: "ready" }], certificates: [{ id: "ready" }] });
  expect(bff.listAyniReleases).toHaveBeenCalledWith(100, expect.anything());
  expect(setHeaders).toHaveBeenCalledWith({ "cache-control": "no-store" });
});
it("rejects oversized input without dispatching a configuration change", async () => {
  const body = new FormData(); body.set("engineImage", "a".repeat(513));
  const result = await actions.save!({ locals, request: new Request("http://localhost/ayni?/save", { method: "POST", body }) } as never);
  expect(result).toMatchObject({ status: 400 });
  expect(bff.updateBuildService).not.toHaveBeenCalled();
});
it("does not expose internal backend error details", async () => {
  bff.getBuildService.mockRejectedValue(new Error("private backend detail"));
  const result = await load({ locals, setHeaders: vi.fn(), url: new URL("http://localhost/ayni") } as never);
  expect(JSON.stringify(result)).not.toContain("private backend detail");
});

it("does not dispatch a submitted security mode as deployment authority", async () => {
  const body = new FormData();
  body.set("securityMode", "dedicated-builder");
  await actions.save!({ locals, request: new Request("http://localhost/ayni?/save", { method: "POST", body }) } as never);
  expect(bff.updateBuildService).toHaveBeenCalledWith(expect.objectContaining({ settings: expect.objectContaining({ securityMode: "" }) }), expect.anything());
});

it("starts installation explicitly and ignores submitted builder identities", async () => {
  const body = new FormData(); body.set("idempotencyKey", "install"); body.set("resourceVersion", "3");
  await actions.install!({ locals, request: new Request("http://localhost/ayni?/install", {method:"POST",body}) } as never);
  expect(bff.startBuilderInstallation).toHaveBeenCalledWith({idempotencyKey:"install",expectedResourceVersion:"3"},expect.anything());
  body.set("builderImage", "malicious/image");
  await actions.save!({ locals, request: new Request("http://localhost/ayni?/save", {method:"POST",body}) } as never);
  expect(bff.updateBuildService).toHaveBeenCalledWith(expect.objectContaining({settings:expect.objectContaining({builderImage:""})}),expect.anything());
});
