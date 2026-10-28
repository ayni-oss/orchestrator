export type Workspace = { id: string; displayName: string; resourceVersion: string };
export type Project = { id: string; displayName: string; workspaceId: string; goal: string; resourceVersion: string };
export type WorkspaceData = { workspace?: Workspace; workspaces: Workspace[]; projects: Project[]; error?: string; nextWorkspace?: string; nextProject?: string };
