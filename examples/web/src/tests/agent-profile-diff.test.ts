import { expect, it } from "vitest";
import { capabilities } from "../../fixtures/agent-capabilities.js";
import { profileChanges } from "$lib/agent-profile-diff.js";
it("treats tools and repository selections as sets when reviewing changes", () => {
  const before = { ...capabilities.roles.planner, repositoryIds: ["a", "b"] };
  const after = { ...before, allowedTools: [...before.allowedTools].reverse(), repositoryIds: ["b", "a"] };
  expect(profileChanges(before, after, false, false, "Application capabilities")).toEqual([]);
});
it("reviews skill instruction changes even when the pinned revision stays the same", () => {
  const before = { ...capabilities.roles.planner, skills: [{ id: "review", revision: "v1", instructions: "Before" }] };
  const after = { ...before, skills: [{ id: "review", revision: "v1", instructions: "After" }] };
  expect(profileChanges(before, after, false, false, "Application capabilities")).toEqual([{ label: "Pinned skills", before: "review @ v1\nBefore", after: "review @ v1\nAfter" }]);
});
it("reviews project instructions as additions to inherited instructions", () => {
  const before = { ...capabilities.roles.planner, instructions: "Workspace guidance" };
  const after = { ...before, instructions: "Project guidance" };
  expect(profileChanges(before, after, true, false, "Workspace capabilities", true)).toContainEqual({ label: "Additional instructions", before: "None", after: "Project guidance" });
});
