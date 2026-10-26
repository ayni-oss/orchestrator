import {capabilities} from "../../fixtures/agent-capabilities.js";
import {describe,it,expect} from "vitest";
import {parseProfile} from "$lib/server/agent-profiles.js";
function form(){const value=new FormData();value.set("planner.calls","4");value.set("planner.deadline","300");value.append("planner.tools","read_repository_file");return value;}
describe("agent profile input",()=>{
 it("preserves explicit inheritance and pins skill content with its revision",()=>{
  const value=form();value.set("planner.inherit","true");expect(parseProfile(value,"planner",capabilities)).toBeUndefined();value.delete("planner.inherit");value.set("planner.skill.0.id","compatibility");value.set("planner.skill.0.revision","v1");value.set("planner.skill.0.instructions","Review compatibility");
  expect(parseProfile(value,"planner",capabilities)).toMatchObject({maxModelCalls:4,deadlineSeconds:"300",skills:[{id:"compatibility",revision:"v1",instructions:"Review compatibility"}]});
 });
 it("rejects mutation tools for planners and invalid budgets or skill revisions",()=>{
  const forbidden=form();forbidden.set("planner.tools","run_command");expect(()=>parseProfile(forbidden,"planner",capabilities)).toThrow();
  const overBudget=form();overBudget.set("planner.calls","129");expect(()=>parseProfile(overBudget,"planner",capabilities)).toThrow();
  const skill=form();skill.set("planner.skill.0.id","review");expect(()=>parseProfile(skill,"planner",capabilities)).toThrow();
 });
});
