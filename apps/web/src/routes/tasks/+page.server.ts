import { redirect } from "@sveltejs/kit";
import type { PageServerLoad } from "./$types";

/** Task history lives in each project's Tasks tab. */
export const load: PageServerLoad = ({ url }) => {
  const project = url.searchParams.get("project");
  if (project) redirect(303, `/projects/${encodeURIComponent(project)}?view=tasks`);
  redirect(303, "/operations");
};
