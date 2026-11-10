import { error as fail } from '@sveltejs/kit';
import type { LayoutServerLoad } from './$types';
import type { Workspace, Project } from '$lib/workspace-types';

export const load: LayoutServerLoad = async ({ fetch, url, params }) => {
  const request = async (method: string, body: unknown) => {
    const response = await fetch(`/api/${method}`, {
      method: 'POST', headers: { 'Content-Type': 'application/json', 'x-console-request': '1' },
      body: JSON.stringify(body),
    });
    if (!response.ok) throw new Error('The local service could not load this workspace.');
    return response.json();
  };
  const selected = params.workspaceId;
  let result;
  try {
    const first: { workspaces?: Workspace[]; nextAfterId?: string } = await request('ListWorkspaces', {
      afterId: selected ? '' : url.searchParams.get('afterWorkspace') ?? '', limit: 50,
    });
    let workspace = first.workspaces?.find(item => item.id === selected);
    let cursor = first.nextAfterId;
    const visited = new Set<string>();
    while (selected && !workspace && cursor) {
      if (visited.has(cursor)) throw new Error('Workspace pagination did not advance.');
      visited.add(cursor);
      const next = await request('ListWorkspaces', { afterId: cursor, limit: 100 });
      workspace = next.workspaces?.find((item: Workspace) => item.id === selected);
      cursor = next.nextAfterId;
    }
    const projects: { projects?: Project[]; nextAfterId?: string } = workspace
      ? await request('ListProjects', { workspaceId: workspace.id, afterId: url.searchParams.get('afterProject') ?? '', limit: 50 })
      : {};
    result = { workspaces: first.workspaces ?? [], workspace, projects: projects.projects ?? [], nextWorkspace: first.nextAfterId ?? '', nextProject: projects.nextAfterId ?? '', error: '' };
  } catch {
    fail(503, 'The local service could not load workspaces. Retry when it is ready.');
  }
  if (selected && !result.workspace) fail(404, 'This workspace was not found.');
  return result;
};
