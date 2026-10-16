import { beforeEach, describe, expect, it, vi } from "vitest";
import { capabilities } from "../../fixtures/agent-capabilities.js";
const bff = vi.hoisted(() => ({ getAgentCapabilities: vi.fn(), getWorkspace: vi.fn(), getProject: vi.fn(), updateWorkspace: vi.fn(), updateProject: vi.fn() }));
vi.mock("$lib/server/bff.js", () => ({ getBff: () => bff }));
import { saveAgentProfile } from "$lib/server/agent-profiles.js";
const defaults = { projectConnectionId: "research", projectModel: "research-model", codingConnectionId: "coding", codingModel: "coding-model", plannerProfile: capabilities.roles.planner, workerProfile: capabilities.roles.worker };
const current = { id: "scope:1", displayName: "Example", resourceVersion: "3", agentDefaults: defaults };
function event(role: string, inherit = false) {
  const form = new FormData();
  form.set("idempotencyKey", "00000000-0000-4000-8000-000000000001"); form.set("resourceVersion", "3"); form.set("role", role);
  if (inherit) form.set(`${role}.inherit`, "true");
  else { form.set(`${role}.calls`, "4"); form.set(`${role}.deadline`, "300"); form.set(`${role}.instructions`, "Review changes"); form.append(`${role}.tools`, role === "planner" ? "read_repository_file" : "run_command"); }
  return { params: { workspaceId: "scope:1", projectId: "scope:1" }, locals: { requestId: "profile-save" }, request: new Request("http://localhost/settings", { method: "POST", body: form }) } as never;
}
beforeEach(() => { vi.resetAllMocks(); bff.getAgentCapabilities.mockResolvedValue(capabilities); bff.getWorkspace.mockResolvedValue(current); bff.getProject.mockResolvedValue(current); });
describe.each(["workspace", "project"] as const)("%s profile saves", scope => {
  it.each(["planner", "worker"] as const)("saves only the %s profile while preserving the other profile and models", async role => {
    await saveAgentProfile(event(role), scope);
    const update = scope === "workspace" ? bff.updateWorkspace : bff.updateProject;
    const other = role === "planner" ? "workerProfile" : "plannerProfile";
    expect(update).toHaveBeenCalledWith(expect.objectContaining({ expectedResourceVersion: "3", agentDefaults: { ...defaults, [`${role}Profile`]: expect.objectContaining({ maxModelCalls: 4, instructions: "Review changes" }), [other]: defaults[other] } }), expect.anything());
  });
  it("restores planner inheritance while keeping the configured worker profile", async () => {
    await saveAgentProfile(event("planner", true), scope);
    const update = scope === "workspace" ? bff.updateWorkspace : bff.updateProject;
    expect(update).toHaveBeenCalledWith(expect.objectContaining({ agentDefaults: { ...defaults, plannerProfile: undefined } }), expect.anything());
  });
});
