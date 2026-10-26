import type { PageServerLoad } from './$types';
import { getBff } from '$lib/server/bff.js';
import { requestContext, safeError } from '$lib/server/page.js';
import { workspaceChoices, workspaceProjects } from '$lib/server/project-choices.js';
import { collectChoices } from '$lib/server/choices.js';

export const load: PageServerLoad = async ({ locals, url, depends, setHeaders }) => {
  depends('app:operations');
  setHeaders({ 'cache-control': 'no-store' });
  const context = requestContext(locals), bff = getBff();
  try {
    const workspaces = await workspaceChoices(context);
    const selected = url.searchParams.get('workspace');
    const workspace = selected ? workspaces.find(w => w.id === selected) : workspaces.find(w => w.state === 'active') ?? workspaces[0];
    if (!workspace) return { workspaces, workspace, projects: [], tasks: [], conversations: [], error: selected ? {message:'This workspace is unavailable. Choose another workspace.'} : undefined };
    const projects = await workspaceProjects(workspace.id, context);
    const results = await Promise.all(projects.map(async project => {
      const [tasks, conversations] = await Promise.all([
        collectChoices(async pageToken => { const result = await bff.listDevelopmentTasks({ pageSize:100, pageToken }, context, project.id); return {items:result.developmentTasks,nextPageToken:result.page.nextPageToken}; }, 10000),
        bff.listProjectConversations(project.id, context),
      ]);
      return { tasks, conversations };
    }));
    return { workspaces, workspace, projects, tasks:results.flatMap(r => r.tasks), conversations:results.flatMap(r => r.conversations), error:undefined };
  } catch (error) {
    return { workspaces:[], workspace:undefined, projects:[], tasks:[], conversations:[], error:safeError(error, locals.requestId) };
  }
};
