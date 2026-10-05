import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError, text } from "$lib/server/page.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";

export const load: PageServerLoad = async ({ locals, params, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  const base = { idempotencyKey: actionKey(), requestId: locals.requestId };
  try {
    const context = requestContext(locals), bff = getBff();
    const project = await bff.getProject(params.projectId, context);
    const repositoryId = project.repositoryConnectionIds[0]!;
    const [repository, profiles, tasks] = await Promise.all([
      bff.getRepositoryConnection(repositoryId, context),
      bff.listRuntimeProfiles(repositoryId, { pageSize: 100 }, context),
      bff.listDevelopmentTasks({ pageSize: 100, pageToken: url.searchParams.get("pageToken") ?? "" }, context, project.id),
    ]);
    return { ...base, project, repository, profiles: profiles.runtimeProfiles, tasks: tasks.developmentTasks, nextPageToken: tasks.page.nextPageToken };
  } catch (error) {
    return { ...base, project: undefined, repository: undefined, profiles: [], tasks: [], nextPageToken: "", error: safeError(error, locals.requestId) };
  }
};

export const actions: Actions = {
  update: async (event) => mutate(event, false),
  archive: async (event) => mutate(event, true),
};

async function mutate({ locals, params, request }: Parameters<NonNullable<Actions[string]>>[0], archive: boolean) {
  const form = await request.formData();
  const idempotencyKey = text(form, "idempotencyKey", 36);
  const expectedResourceVersion = text(form, "resourceVersion", 20);
  const displayName = text(form, "displayName", 256);
  if (!idempotencyKey || !isCanonicalIdentifier(idempotencyKey) || !expectedResourceVersion || !/^[1-9][0-9]{0,18}$/u.test(expectedResourceVersion) || (!archive && !displayName)) return fail(400, { error: { message: "Review the project settings and reload if they have changed.", requestId: locals.requestId } });
  try {
    const input = { idempotencyKey, projectId: params.projectId!, expectedResourceVersion };
    if (archive) await getBff().archiveProject(input, requestContext(locals));
    else await getBff().updateProject({ ...input, displayName: displayName! }, requestContext(locals));
    return { success: archive ? "Project archived. History is retained." : "Project updated." };
  } catch (error) {
    const mapped = safeError(error, locals.requestId);
    return fail(mapped.status, { error: mapped });
  }
}
