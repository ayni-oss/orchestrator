import { fail, redirect } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError, text } from "$lib/server/page.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";

export const load: PageServerLoad = async ({ locals, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  const context = requestContext(locals);
  const [projects, repositories] = await Promise.allSettled([
    getBff().listProjects({ pageSize: 100, pageToken: url.searchParams.get("pageToken") ?? "" }, context),
    getBff().listRepositoryConnections({ pageSize: 100, pageToken: url.searchParams.get("repositoryPageToken") ?? "" }, context),
  ]);
  const failure = projects.status === "rejected" ? projects.reason : repositories.status === "rejected" ? repositories.reason : undefined;
  return {
    projects: projects.status === "fulfilled" ? projects.value.projects : [],
    nextPageToken: projects.status === "fulfilled" ? projects.value.page.nextPageToken : "",
    repositories: repositories.status === "fulfilled" ? repositories.value.repositoryConnections : [],
    repositoryPageToken: repositories.status === "fulfilled" ? repositories.value.page.nextPageToken : "",
    idempotencyKey: actionKey(), requestId: locals.requestId,
    ...(failure === undefined ? {} : { error: safeError(failure, locals.requestId) }),
  };
};

export const actions: Actions = {
  create: async ({ locals, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    const displayName = text(form, "displayName", 256);
    const repositoryConnectionId = text(form, "repositoryConnectionId", 128);
    if (!idempotencyKey || !isCanonicalIdentifier(idempotencyKey) || !displayName || !repositoryConnectionId) return fail(400, { error: { message: "Enter a project name and select its repository.", requestId: locals.requestId } });
    let id: string;
    try {
      id = (await getBff().createProject({ idempotencyKey, displayName, repositoryConnectionId }, requestContext(locals))).id;
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { error: mapped });
    }
    redirect(303, `/projects/${encodeURIComponent(id)}`);
  },
};
