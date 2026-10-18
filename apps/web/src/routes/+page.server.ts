import { fail, redirect } from "@sveltejs/kit";
import { superValidate, message } from "sveltekit-superforms";
import { zod } from "sveltekit-superforms/adapters";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError } from "$lib/server/page.js";
import { workspaceChoices, workspaceProjects, projectRepositoryChoices } from "$lib/server/project-choices.js";
import { projectSchema } from "$lib/project-schema.js";

export const load: PageServerLoad = async ({ locals, url, depends, setHeaders }) => {
  depends("app:workspaces", "app:projects", "app:repositories");
  setHeaders({ "cache-control": "no-store" });
  const context = requestContext(locals);
  const selected = url.searchParams.get("workspace");
  const form = await superValidate({ idempotencyKey: actionKey() }, zod(projectSchema), { errors: false });
  try {
    const workspaces = await workspaceChoices(context);
    const workspace = selected ? workspaces.find(item => item.id === selected) : workspaces.find(item => item.state === "active") ?? workspaces[0];
    if (!workspace) return { workspaces, workspace: undefined, projects: [], repositories: [], form, error: selected ? { message: "This workspace is unavailable. Choose another workspace." } : undefined };
    const [projects, repositories] = await Promise.all([workspaceProjects(workspace.id, context), projectRepositoryChoices(context)]);
    form.data.workspaceId = workspace.id;
    return { workspaces, workspace, projects, repositories, form, error: undefined };
  } catch (error) {
    return { workspaces: [], workspace: undefined, projects: [], repositories: [], form, error: safeError(error, locals.requestId) };
  }
};

export const actions: Actions = {
  create: async ({ locals, request, url }) => {
    const form = await superValidate(request, zod(projectSchema));
    if (!form.valid) return fail(400, { form });
    let id: string;
    try {
      const context = requestContext(locals);
      const workspace = await getBff().getWorkspace(form.data.workspaceId, context);
      if ((url.searchParams.has("workspace") && url.searchParams.get("workspace") !== workspace.id) || workspace.state !== "active" || form.data.repositoryConnectionIds.some(id => !workspace.repositoryConnectionIds.includes(id))) {
        return message(form, "Choose repositories in the selected active workspace.", { status: 400 });
      }
      id = (await getBff().createProject(form.data, context)).id;
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return message(form, mapped.message, { status: mapped.status as 400 });
    }
    redirect(303, `/_old/projects/${encodeURIComponent(id)}`);
  },
};
