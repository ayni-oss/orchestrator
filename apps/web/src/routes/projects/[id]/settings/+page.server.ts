import type { Project } from '$lib/workspace-types';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ fetch, params, parent, url }) => {
  const { workspace, projects, nextProject } = await parent();
  const project = projects.find(item => item.id === params.id);
  if (project || !workspace) return { project };

  let afterId = url.searchParams.has('afterProject') ? '' : nextProject;
  if (!afterId && !url.searchParams.has('afterProject')) return { project: undefined };

  do {
    const response = await fetch('/api/ListProjects', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'x-console-request': '1' },
      body: JSON.stringify({ workspaceId: workspace.id, afterId, limit: 100 }),
    });
    if (!response.ok) break;
    const result: { projects?: Project[]; nextAfterId?: string } = await response.json();
    const match = result.projects?.find(item => item.id === params.id);
    if (match) return { project: match };
    if (afterId === result.nextAfterId) break;
    afterId = result.nextAfterId ?? '';
  } while (afterId);

  return { project: undefined };
};
