import { goto } from '$app/navigation';

export function hierarchyActions(kind: 'workspace' | 'project', id: string, rename: () => void, workspaceId?: string) {
  const scopeAction = kind === 'workspace' ? {
    label: 'Workspace settings',
    onSelect: () => { void goto(`/workspace/settings?workspace=${id}`); },
  } : {
    label: 'Repositories',
    onSelect: () => { void goto(`/projects/${id}/repositories?workspace=${workspaceId}`); },
  };
  return [scopeAction, { label: 'Rename', onSelect: rename }];
}
