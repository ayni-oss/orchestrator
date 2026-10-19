import type { RequestContext } from "@ayni-orchestrator/bff/server";
import { workspaceChoices, workspaceProjects } from "$lib/server/project-choices.js";

/** Resolve the workspace navigation for pages that are not themselves workspace routes. */
export async function shellSelection(context: RequestContext, preferredWorkspaceId?: string, repositoryId?: string) {
  const workspaces = await workspaceChoices(context).catch(() => []);
  const workspace = workspaces.find(item => item.id === preferredWorkspaceId)
    ?? workspaces.find(item => repositoryId && item.repositoryConnectionIds.includes(repositoryId))
    ?? workspaces.find(item => item.state === "active")
    ?? workspaces[0];
  const projects = workspace ? await workspaceProjects(workspace.id, context).catch(() => []) : [];
  return { workspaces, workspace, projects };
}
