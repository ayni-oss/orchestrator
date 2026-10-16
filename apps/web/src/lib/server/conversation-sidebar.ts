import type { RequestContext } from "@ayni-orchestrator/bff/server";
import type { ConversationSidebarData } from "$lib/conversation-sidebar.js";
import { getBff } from "./bff.js";
import { projectRepositoryChoices, workspaceProjects } from "./project-choices.js";

export async function loadConversationSidebar(params: { workspaceId?: string | undefined; projectId?: string | undefined; taskId?: string | undefined }, context: RequestContext): Promise<ConversationSidebarData | undefined> {
  const bff = getBff();
  let workspaceId = params.workspaceId;
  const projectId = params.projectId ?? (params.taskId ? (await bff.getDevelopmentTask(params.taskId, context)).projectId : undefined);
  if (!workspaceId && projectId) workspaceId = (await bff.getProject(projectId, context)).workspaceId;
  if (!workspaceId) return undefined;
  const [workspace, projects, repositories] = await Promise.all([bff.getWorkspace(workspaceId, context), workspaceProjects(workspaceId, context), projectRepositoryChoices(context)]);
  const results = await Promise.allSettled(projects.map(async project => (await bff.listProjectConversations(project.id, context)).filter(conversation => conversation.projectId === project.id).map(conversation => ({ id: conversation.id, projectId: project.id, projectName: project.displayName, title: conversation.title, createdAt: conversation.createdAt }))));
  return {
    workspace: { id: workspace.id, displayName: workspace.displayName, state: workspace.state },
    projects: projects.map(project => ({ id: project.id, displayName: project.displayName, state: project.state, repositoryConnectionIds: project.repositoryConnectionIds.filter(id => workspace.repositoryConnectionIds.includes(id)) })),
    repositories: repositories.filter(repository => workspace.repositoryConnectionIds.includes(repository.id)).map(repository => ({ id: repository.id, displayName: repository.displayName })),
    conversations: results.flatMap(result => result.status === "fulfilled" ? result.value : []).sort((a, b) => b.createdAt.localeCompare(a.createdAt) || a.id.localeCompare(b.id)),
    incomplete: results.some(result => result.status === "rejected"),
  };
}
