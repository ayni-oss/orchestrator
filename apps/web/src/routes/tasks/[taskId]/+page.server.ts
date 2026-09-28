import type { DevelopmentTaskDto } from "@ayni-orchestrator/bff/server";
import { fail, redirect } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";
import { actionKey, requestContext, safeError, text } from "$lib/server/page.js";

export const load: PageServerLoad = async ({ locals, params, setHeaders }) => {
  setHeaders({ "cache-control": "no-store" });
  const actionKeys = { cancel: actionKey(), retry: actionKey(), provideInput: actionKey() };
  if (!isCanonicalIdentifier(params.taskId)) return { task: undefined, evidence: undefined, requestId: locals.requestId, actionKeys, error: { status: 400, reason: "invalid_identifier", message: "The task identifier is invalid.", requestId: locals.requestId, retryable: false } };
  const context = requestContext(locals);
  try {
    const task = await getBff().getDevelopmentTask(params.taskId, context);
    let evidence;
    let evidenceError;
    try {
      evidence = await getBff().getDevelopmentTaskEvidence(params.taskId, context);
    } catch (error) {
      const mapped = safeError(error, locals.requestId, "retained evidence could not be loaded");
      if (mapped.status !== 404) evidenceError = mapped;
    }
    return { task, evidence, evidenceError, requestId: locals.requestId, actionKeys };
  } catch (error) {
    return { task: undefined, evidence: undefined, requestId: locals.requestId, actionKeys, error: safeError(error, locals.requestId) };
  }
};

async function mutate(kind: "cancel" | "retry", idempotencyKey: string, taskId: string, locals: App.Locals): Promise<DevelopmentTaskDto> {
  const input = { idempotencyKey, taskId };
  return kind === "cancel" ? getBff().cancelDevelopmentTask(input, requestContext(locals)) : getBff().retryDevelopmentTask(input, requestContext(locals));
}

export const actions: Actions = {
  cancel: async ({ locals, params, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    if (!isCanonicalIdentifier(params.taskId) || idempotencyKey === undefined || !isCanonicalIdentifier(idempotencyKey)) return fail(400, { action: "cancel", error: { status: 400, reason: "invalid_identifier", message: "The task identifier is invalid.", requestId: locals.requestId, retryable: false } });
    try { await mutate("cancel", idempotencyKey, params.taskId, locals); } catch (error) { const mapped = safeError(error, locals.requestId); return fail(mapped.status, { action: "cancel", error: mapped }); }
    redirect(303, `/tasks/${params.taskId}?notice=cancelled`);
  },
  retry: async ({ locals, params, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    if (!isCanonicalIdentifier(params.taskId) || idempotencyKey === undefined || !isCanonicalIdentifier(idempotencyKey)) return fail(400, { action: "retry", error: { status: 400, reason: "invalid_identifier", message: "The task identifier is invalid.", requestId: locals.requestId, retryable: false } });
    let task: DevelopmentTaskDto;
    try { task = await mutate("retry", idempotencyKey, params.taskId, locals); } catch (error) { const mapped = safeError(error, locals.requestId); return fail(mapped.status, { action: "retry", error: mapped }); }
    redirect(303, `/tasks/${task.id}?notice=retried`);
  },
  provideInput: async ({ locals, params, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    const input = text(form, "input", 8_192);
    if (!isCanonicalIdentifier(params.taskId) || idempotencyKey === undefined || !isCanonicalIdentifier(idempotencyKey) || input === undefined) return fail(400, { action: "provideInput", error: { status: 400, reason: "invalid_input", message: "Provide a non-empty response of at most 8192 characters.", requestId: locals.requestId, retryable: false } });
    let task: DevelopmentTaskDto;
    try { task = await getBff().provideDevelopmentTaskInput({ idempotencyKey, taskId: params.taskId, input }, requestContext(locals)); } catch (error) { const mapped = safeError(error, locals.requestId); return fail(mapped.status, { action: "provideInput", error: mapped }); }
    redirect(303, `/tasks/${task.id}?notice=input-provided`);
  },
};
