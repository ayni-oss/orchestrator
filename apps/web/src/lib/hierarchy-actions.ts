import { goto } from '$app/navigation';

export function workspaceActions(id: string) {
  return [{
    label: 'Workspace settings',
    onSelect: () => { void goto(`/workspace/settings?workspace=${id}`); },
  }];
}

export function projectActions(id: string, workspaceId: string | undefined, rename: () => void) {
  return [{
    label: 'Repositories',
    onSelect: () => { void goto(`/projects/${id}/repositories?workspace=${workspaceId}`); },
  }, { label: 'Rename', onSelect: rename }];
}
