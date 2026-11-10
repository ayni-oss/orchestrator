import { projectSettingsPath, workspaceSettingsPath } from '$lib/paths';
import { goto } from '$app/navigation';

export function workspaceActions(id: string) {
  return [{
    label: 'Workspace settings',
    onSelect: () => { void goto(workspaceSettingsPath(id)); },
  }];
}

export function projectActions(id: string, workspaceId: string | undefined) {
  if (!workspaceId) return [];
  return [{
    label: 'Project settings',
    onSelect: () => { void goto(projectSettingsPath(workspaceId, id)); },
  }];
}
