import {describe,expect,it} from "vitest";
import {effectiveAgentDefaults} from "$lib/server/agent-defaults.js";
const workspace={projectConnectionId:"shared",projectModel:"research",codingConnectionId:"shared",codingModel:"coding"};
describe("workspace model inheritance",()=>{
  it("restores workspace defaults after clearing project overrides",()=>{
    expect(effectiveAgentDefaults({projectConnectionId:"",projectModel:"",codingConnectionId:"",codingModel:""},workspace)).toEqual(workspace);
  });
  it("inherits each connection and model pair independently",()=>{
    expect(effectiveAgentDefaults({projectConnectionId:"custom",projectModel:"custom-research",codingConnectionId:"",codingModel:""},workspace)).toEqual({...workspace,projectConnectionId:"custom",projectModel:"custom-research"});
  });
  it("leaves unconfigured agents unset",()=>{expect(effectiveAgentDefaults(undefined,undefined)).toBeUndefined();});
});
