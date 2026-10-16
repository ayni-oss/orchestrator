import type { AgentProfileSettingsDto } from "$lib/project-view.js";

export function profileChanges(before: AgentProfileSettingsDto, after: AgentProfileSettingsDto, inheritedBefore: boolean, inheritedAfter: boolean, source: string, project = false) {
  const tools = (profile: AgentProfileSettingsDto) => [...profile.allowedTools].sort().join("\n") || "None";
  const repositories = (profile: AgentProfileSettingsDto) => [...profile.repositoryIds].sort().join("\n") || "Current repository membership";
  const skills = (profile: AgentProfileSettingsDto) => [...profile.skills].sort((a, b) => a.id.localeCompare(b.id)).map(skill => `${skill.id} @ ${skill.revision}\n${skill.instructions}`).join("\n\n") || "None";
  return [
    { label: "Settings source", before: inheritedBefore ? source : "Custom settings", after: inheritedAfter ? source : "Custom settings" },
    { label: project ? "Additional instructions" : "Instructions", before: (project && inheritedBefore ? "" : before.instructions) || (project ? "None" : "Application role instructions"), after: (project && inheritedAfter ? "" : after.instructions) || (project ? "None" : "Application role instructions") },
    { label: "Maximum model calls", before: String(before.maxModelCalls), after: String(after.maxModelCalls) },
    { label: "Maximum duration (seconds)", before: before.deadlineSeconds, after: after.deadlineSeconds },
    { label: "Allowed tools", before: tools(before), after: tools(after) },
    { label: "Repository scope", before: repositories(before), after: repositories(after) },
    { label: "Pinned skills", before: skills(before), after: skills(after) },
  ].filter(change => change.before !== change.after);
}
