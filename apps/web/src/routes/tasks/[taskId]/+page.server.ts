import { fail, redirect } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";
import { shellSelection } from "$lib/server/shell-selection.js";
import { actionKey, requestContext, safeError, text } from "$lib/server/page.js";

const RESOURCE_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;

export const load: PageServerLoad = async ({ locals, params, depends,setHeaders }) => {
  depends("app:tasks");
  setHeaders({ "cache-control": "no-store" });
  const context = requestContext(locals);
  const actionKeys = { cancel: actionKey(), retry: actionKey(), retryPublication: actionKey() };
  if (!RESOURCE_ID.test(params.taskId)) return { ...await shellSelection(context), task: undefined, requestId: locals.requestId, actionKeys, error: { status: 400, reason: "invalid_identifier", message: "The task identifier is invalid.", requestId: locals.requestId, retryable: false } };
  try {
    const task = await getBff().getDevelopmentTask(params.taskId, requestContext(locals));
    const progress=await getBff().taskProgress(params.taskId,context);
    const project=await getBff().getProject(task.projectId,context).catch(()=>undefined);
    const shell=await shellSelection(context,project?.workspaceId);
    return { ...shell, task, progress, requestId: locals.requestId, actionKeys };
  } catch (error) {
    return { ...await shellSelection(context), task: undefined, requestId: locals.requestId, actionKeys, error: safeError(error, locals.requestId) };
  }
};

function validMutation(taskId: string, key: string | undefined): key is string {
  return RESOURCE_ID.test(taskId) && key !== undefined && isCanonicalIdentifier(key);
}

export const actions: Actions = {
  retryPublication: async ({ locals, params, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    if (!validMutation(params.taskId, idempotencyKey)) return fail(400, { action: "retryPublication", error: { status: 400, reason: "invalid_identifier", message: "The task identifier is invalid.", requestId: locals.requestId, retryable: false } });
    try {
      await getBff().retryTaskPublication({ idempotencyKey, developmentTaskId: params.taskId }, requestContext(locals));
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "retryPublication", error: mapped });
    }
    redirect(303, `/tasks/${encodeURIComponent(params.taskId)}?notice=publication-retried`);
  },
  cancel: async ({ locals, params, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    if (!validMutation(params.taskId, idempotencyKey)) return fail(400, { action: "cancel", error: { status: 400, reason: "invalid_identifier", message: "The task identifier is invalid.", requestId: locals.requestId, retryable: false } });
    try {
      await getBff().cancelDevelopmentTask({ idempotencyKey, developmentTaskId: params.taskId }, requestContext(locals));
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "cancel", error: mapped });
    }
    redirect(303, `/tasks/${encodeURIComponent(params.taskId)}?notice=cancelled`);
  },
  retry: async ({ locals, params, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    if (!validMutation(params.taskId, idempotencyKey)) return fail(400, { action: "retry", error: { status: 400, reason: "invalid_identifier", message: "The task identifier is invalid.", requestId: locals.requestId, retryable: false } });
    try {
      await getBff().retryDevelopmentTask({ idempotencyKey, developmentTaskId: params.taskId }, requestContext(locals));
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "retry", error: mapped });
    }
    redirect(303, `/tasks/${encodeURIComponent(params.taskId)}?notice=retried`);
  },
};
