export const projectViews = ["overview", "plans", "tasks"] as const;
export type ProjectView = (typeof projectViews)[number];

export function projectView(value: string | null): ProjectView {
  return projectViews.find(view => view === value) ?? "overview";
}

/** A view change keeps the conversation, but starts a fresh task page. */
export function projectHref(projectId: string, view: ProjectView, conversationId?: string, pageToken?: string, version?: string) {
  const query = new URLSearchParams({ view });
  if (conversationId) query.set("conversation", conversationId);
  if (view === "tasks" && pageToken) query.set("pageToken", pageToken);
  if (view === "plans" && version) query.set("version", version);
  return `/projects/${encodeURIComponent(projectId)}?${query}`;
}
