import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError, text } from "$lib/server/page.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";
import { projectRepositoryChoices } from "$lib/server/project-choices.js";
import { projectSchema } from "$lib/project-schema.js";
import { actions as taskActions } from "../../tasks/+page.server.js";
export const load: PageServerLoad = async ({ locals, params, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  const base = { idempotencyKey: actionKey(), taskKey: actionKey(), requestId: locals.requestId };
  try {
    const context = requestContext(locals), bff = getBff();
    const project = await bff.getProject(params.projectId, context);
    const [repositories, profiles, tasks, choices, models] = await Promise.all([
      Promise.all(project.repositoryConnectionIds.map(id => bff.getRepositoryConnection(id, context))),
      Promise.all(project.repositoryConnectionIds.map(id => bff.listRuntimeProfiles(id, { pageSize: 100 }, context))),
      bff.listDevelopmentTasks({ pageSize: 100, pageToken: url.searchParams.get("pageToken") ?? "" }, context, project.id),
      projectRepositoryChoices(context),
      bff.listProviderModels({pageSize:100},context).catch(() => undefined),
    ]);
    return { ...base, project, repositories, profiles: profiles.flatMap(page => page.runtimeProfiles), tasks: tasks.developmentTasks, nextPageToken: tasks.page.nextPageToken, choices, models:models?.models ?? [], provider:models?.provider ?? "" };
  } catch (error) {
    return { ...base, project: undefined, repositories: [], profiles: [], tasks: [], nextPageToken: "", choices:[], models:[], provider:"", error: safeError(error, locals.requestId) };
  }
};
export const actions: Actions = {
  update: async (event) => mutate(event, false),
  archive: async (event) => mutate(event, true),
  createTask: async event => {
    const form = await event.request.clone().formData();
    if (form.get("projectId") !== event.params.projectId) return fail(400, {error:{message:"Select this project to submit work.",requestId:event.locals.requestId}});
    return taskActions.create!(event as never);
  },
};
async function mutate({ locals, params, request }: Parameters<NonNullable<Actions[string]>>[0], archive: boolean) {
  const form = await request.formData();
  const idempotencyKey = text(form, "idempotencyKey", 36);
  const expectedResourceVersion = text(form, "resourceVersion", 20);
  const parsed = projectSchema.safeParse({idempotencyKey,displayName:form.get("displayName"),repositoryConnectionIds:form.getAll("repositoryConnectionIds")});
  if (!idempotencyKey || !isCanonicalIdentifier(idempotencyKey) || !expectedResourceVersion || !/^[1-9][0-9]{0,18}$/u.test(expectedResourceVersion) || (!archive && !parsed.success)) return fail(400, { error: { message: "Enter a project name, select at least one repository, and reload if settings have changed.", requestId: locals.requestId } });
  try {
    const input = { idempotencyKey, projectId: params.projectId!, expectedResourceVersion };
    if (archive) await getBff().archiveProject(input, requestContext(locals));
    else if (parsed.success) await getBff().updateProject({ ...input, displayName: parsed.data.displayName, repositoryConnectionIds:parsed.data.repositoryConnectionIds }, requestContext(locals));
    return { success: archive ? "Project archived. History is retained." : "Project updated." };
  } catch (error) {
    const mapped = safeError(error, locals.requestId);
    return fail(mapped.status, { error: mapped });
  }
}
