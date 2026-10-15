import {beforeEach,expect,it,vi} from "vitest";
import {capabilities} from "../../fixtures/agent-capabilities.js";
const bff=vi.hoisted(()=>({getAgentCapabilities:vi.fn(),getProjectConfiguration:vi.fn(),getWorkspaceConfiguration:vi.fn()}));
vi.mock("$lib/server/bff.js",()=>({getBff:()=>bff}));
import {loadAgentConfiguration} from "$lib/server/agent-configuration.js";
beforeEach(()=>vi.resetAllMocks());
it("keeps raw overrides, inherited settings and server-composed instructions separate",async()=>{
 const configured={plannerProfile:{...capabilities.roles.planner,instructions:"Project"}},inherited={plannerProfile:{...capabilities.roles.planner,instructions:"Workspace"}},effective={plannerProfile:{...capabilities.roles.planner,instructions:"Workspace\nProject"}};
 bff.getAgentCapabilities.mockResolvedValue(capabilities);bff.getProjectConfiguration.mockResolvedValue({project:{workspaceId:"w",agentDefaults:configured},effective});bff.getWorkspaceConfiguration.mockResolvedValue({effective:inherited});
 const value=await loadAgentConfiguration("project","p",{requestId:"config"});expect(value.agentConfiguration).toEqual({configured,inherited,effective});expect(value.capabilities).toEqual(capabilities);
});
it("does not invent capabilities or effective defaults when metadata fails",async()=>{
 bff.getAgentCapabilities.mockRejectedValue(new Error("offline"));const result=await loadAgentConfiguration("project","p",{requestId:"config"});expect(result.agentConfiguration).toBeUndefined();expect(result.capabilities).toBeUndefined();expect(result.configurationError?.message).toContain("reload");expect(bff.getProjectConfiguration).not.toHaveBeenCalled();
});
