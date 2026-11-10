/** Canonical application links: scope belongs in the path; cursors belong in the query. */
function segment(value: string): string {
  if (!value) throw new Error('A resource ID is required.');
  return encodeURIComponent(value);
}
export const workspacePath = (workspaceId: string) => `/workspaces/${segment(workspaceId)}`;
export const workspaceSettingsPath = (workspaceId: string) => `${workspacePath(workspaceId)}/settings`;
export const projectSettingsPath = (workspaceId: string, projectId: string) => `${workspacePath(workspaceId)}/projects/${segment(projectId)}/settings`;
export const repositoryPath = (repositoryId: string) => `/repositories/${segment(repositoryId)}`;
export function withCursor(path: string, name: 'afterWorkspace' | 'afterProject', cursor: string): string {
  return cursor ? `${path}?${new URLSearchParams({ [name]: cursor })}` : path;
}
