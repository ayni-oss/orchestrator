import { goto } from '$app/navigation';

export function workspaceActions(id: string) {
  return [{
    label: 'Workspace settings',
    onSelect: () => { void goto(`/workspace/settings?workspace=${id}`); },
  }];
}

export function projectActions(id: string, workspaceId: string | undefined) {
  return [{
    label: 'Project settings',
    onSelect: () => { void goto(`/projects/${id}/settings?workspace=${workspaceId}`); },
  }];
}
