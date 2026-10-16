export interface ConversationSidebarData {
  workspace: { id: string; displayName: string; state: "active" | "archived" };
  projects: Array<{ id: string; displayName: string; state: "active" | "archived"; repositoryConnectionIds: string[] }>;
  repositories: Array<{ id: string; displayName: string }>;
  conversations: Array<{ id: string; projectId: string; projectName: string; title: string; createdAt: string }>;
  incomplete: boolean;
}
