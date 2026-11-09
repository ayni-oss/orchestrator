import { goto } from '$app/navigation';

export function hierarchyActions(kind: 'workspace' | 'project', id: string, rename: () => void, workspaceId?: string) {
  const actions = [{
    label: 'Repositories',
    onSelect: () => { void goto(kind === 'project' ? `/projects/${id}/repositories?workspace=${workspaceId}` : '/repositories'); },
  }];

  if (kind === 'workspace') {
    actions.push({
      label: 'Workspace settings',
      onSelect: () => { void goto(`/workspace/settings?workspace=${id}`); },
    });
  }

  actions.push({ label: 'Rename', onSelect: rename });
  return actions;
}
