import type { PageServerLoad } from './$types';
import { getBff } from '$lib/server/bff.js';
import { requestContext, safeError } from '$lib/server/page.js';
import { workspaceChoices, workspaceProjects } from '$lib/server/project-choices.js';
import { collectChoices } from '$lib/server/choices.js';

type ActivityEntry = {
  id: string;
  at: string;
  kind: 'repository' | 'conversation' | 'plan' | 'task';
  title: string;
  detail: string;
  context: string;
  href: string;
};

export const load: PageServerLoad = async ({ locals, url, depends, setHeaders }) => {
  depends('app:activity', 'app:workspaces', 'app:projects', 'app:repositories', 'app:tasks', 'app:conversations');
  setHeaders({ 'cache-control': 'no-store' });
  const context = requestContext(locals), bff = getBff();
  try {
    const workspaces = await workspaceChoices(context);
    const selected = url.searchParams.get('workspace');
    const workspace = selected ? workspaces.find(item => item.id === selected) : workspaces.find(item => item.state === 'active') ?? workspaces[0];
    if (!workspace) return { workspaces, workspace, projects: [], entries: [] as ActivityEntry[], error: selected ? { message: 'This workspace is unavailable. Choose another workspace.' } : undefined };

    const [projects, repositories] = await Promise.all([
      workspaceProjects(workspace.id, context),
      Promise.all(workspace.repositoryConnectionIds.map(id => bff.getRepositoryConnection(id, context))),
    ]);
    const projectResults = await Promise.all(projects.map(async project => {
      const [tasks, conversations] = await Promise.all([
        collectChoices(async pageToken => {
          const result = await bff.listDevelopmentTasks({ pageSize: 100, pageToken }, context, project.id);
          return { items: result.developmentTasks, nextPageToken: result.page.nextPageToken };
        }, 10000),
        bff.listProjectConversations(project.id, context),
      ]);
      return { project, tasks, conversations };
    }));

    const entries: ActivityEntry[] = [];
    for (const repository of repositories) {
      entries.push({ id: `repository:${repository.id}`, at: repository.updatedAt, kind: 'repository', title: 'Repository last updated', detail: `${repository.displayName} · current status: ${repository.status.replaceAll('_', ' ')}`, context: workspace.displayName, href: `/repositories/${encodeURIComponent(repository.id)}` });
    }
    for (const { project, tasks, conversations } of projectResults) {
      for (const task of tasks) {
        entries.push({ id: `task:${task.id}:created`, at: task.createdAt, kind: 'task', title: 'Task created', detail: task.snapshot.request, context: project.displayName, href: `/tasks/${encodeURIComponent(task.id)}` });
        if (task.updatedAt !== task.createdAt) entries.push({ id: `task:${task.id}:updated`, at: task.updatedAt, kind: 'task', title: 'Task last updated', detail: `${task.snapshot.request} · current state: ${task.state.replaceAll('_', ' ')}`, context: project.displayName, href: `/tasks/${encodeURIComponent(task.id)}` });
      }
      for (const conversation of conversations) {
        const href = `/projects/${encodeURIComponent(project.id)}?conversation=${encodeURIComponent(conversation.id)}`;
        entries.push({ id: `conversation:${conversation.id}:created`, at: conversation.createdAt, kind: 'conversation', title: 'Conversation started', detail: conversation.title, context: project.displayName, href });
        for (const message of conversation.messages) {
          if (message.role !== 'user' && message.role !== 'assistant') continue;
          entries.push({ id: `message:${message.id}`, at: message.createdAt, kind: 'conversation', title: message.role === 'user' ? 'You sent a message' : 'Project agent replied', detail: message.content, context: project.displayName, href });
        }
        for (const version of conversation.plan?.versions ?? []) {
          entries.push({ id: `plan:${conversation.id}:${version.version}`, at: version.createdAt, kind: 'plan', title: 'Plan version prepared', detail: conversation.title, context: project.displayName, href: `${href}&view=plans` });
        }
      }
    }
    entries.sort((left, right) => Date.parse(right.at) - Date.parse(left.at) || left.id.localeCompare(right.id));
    return { workspaces, workspace, projects, entries: entries.slice(0, 100), error: undefined };
  } catch (error) {
    return { workspaces: [], workspace: undefined, projects: [], entries: [] as ActivityEntry[], error: safeError(error, locals.requestId) };
  }
};
