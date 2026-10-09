import { fail, redirect } from "@sveltejs/kit";
import { superValidate, message } from "sveltekit-superforms";
import { zod } from "sveltekit-superforms/adapters";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError } from "$lib/server/page.js";
import { projectRepositoryChoices } from "$lib/server/project-choices.js";
import { projectSchema } from "$lib/project-schema.js";
export const load: PageServerLoad = async ({ locals, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  const context = requestContext(locals);
  const [projects, repositories] = await Promise.allSettled([
    getBff().listProjectSummaries({ pageSize: 100, pageToken: url.searchParams.get("pageToken") ?? "" }, context),
    projectRepositoryChoices(context),
  ]);
  const failure = [projects, repositories].find(result => result.status === "rejected");
  return {
    summaries: projects.status === "fulfilled" ? projects.value.summaries : [],
    available: projects.status === "fulfilled",
    nextPageToken: projects.status === "fulfilled" ? projects.value.page.nextPageToken : "",
    repositories: repositories.status === "fulfilled" ? repositories.value : [],
    form: await superValidate({idempotencyKey:actionKey()}, zod(projectSchema)),
    requestId: locals.requestId,
    ...(failure?.status === "rejected" ? { error: safeError(failure.reason, locals.requestId) } : {}),
  };
};
export const actions: Actions = {
  create: async ({ locals, request }) => {
    const form = await superValidate(request, zod(projectSchema));
    if (!form.valid) return fail(400, {form});
    let id: string;
    try {
      id = (await getBff().createProject(form.data, requestContext(locals))).id;
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return message(form, mapped.message, {status:mapped.status as 400});
    }
    redirect(303, `/projects/${encodeURIComponent(id)}`);
  },
};
