import { error } from '@sveltejs/kit';
import type { Project } from '$lib/workspace-types';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ fetch, params, parent, url }) => {
  const { workspace, projects, nextProject } = await parent();
  if (!workspace) error(404, 'This workspace was not found.');
  const project = projects.find(item => item.id === params.id);
  if (project) return { project };
  let afterId = url.searchParams.has('afterProject') ? '' : nextProject;
  if (!afterId && !url.searchParams.has('afterProject')) error(404, 'This project was not found in this workspace.');
  const visited = new Set<string>();
  do {
    if (visited.has(afterId)) error(503, 'Project pagination did not advance.');
    visited.add(afterId);
    const response = await fetch('/api/ListProjects', {
      method: 'POST', headers: { 'Content-Type': 'application/json', 'x-console-request': '1' },
      body: JSON.stringify({ workspaceId: workspace.id, afterId, limit: 100 }),
    });
    if (!response.ok) error(503, 'Projects could not be loaded. Retry when the service is ready.');
    const result: { projects?: Project[]; nextAfterId?: string } = await response.json();
    const match = result.projects?.find(item => item.id === params.id);
    if (match) return { project: match };
    afterId = result.nextAfterId ?? '';
  } while (afterId);
  error(404, 'This project was not found in this workspace.');
};
