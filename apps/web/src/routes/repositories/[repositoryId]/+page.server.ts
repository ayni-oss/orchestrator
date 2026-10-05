import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError, text } from "$lib/server/page.js";

const RESOURCE_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;

export const load: PageServerLoad = async ({ locals, params, setHeaders }) => {
  setHeaders({ "cache-control": "no-store" });
  if (!RESOURCE_ID.test(params.repositoryId)) {
    return { repositoryConnection: undefined, runtimeProfiles: [], runtimeKeys: {}, requestId: locals.requestId, error: { status: 400, reason: "invalid_identifier", message: "The repository connection identifier is invalid.", requestId: locals.requestId, retryable: false } };
  }
  const context = requestContext(locals);
  const [repository, profiles] = await Promise.allSettled([
    getBff().getRepositoryConnection(params.repositoryId, context),
    getBff().listRuntimeProfiles(params.repositoryId, { pageSize: 100 }, context),
  ]);
  const failure = repository.status === "rejected" ? repository.reason : profiles.status === "rejected" ? profiles.reason : undefined;
  return {
    repositoryConnection: repository.status === "fulfilled" ? repository.value : undefined,
    runtimeProfiles: profiles.status === "fulfilled" ? profiles.value.runtimeProfiles : [],
    runtimeKeys: profiles.status === "fulfilled" ? Object.fromEntries(profiles.value.runtimeProfiles.map((profile) => [profile.id, actionKey()])) : {},
    requestId: locals.requestId,
    ...(failure === undefined ? {} : { error: safeError(failure, locals.requestId) }),
  };
};

async function reconcile(event: Parameters<Actions[string]>[0], action: "sync" | "rebuild") {
  const form = await event.request.formData();
  const profileId = text(form, "profileId", 128);
  const idempotencyKey = text(form, "idempotencyKey", 36);
  if (!profileId || !idempotencyKey || !/^[0-9a-f-]{36}$/u.test(idempotencyKey)) return fail(400, { error: { message: "Refresh and try again." } });
  try {
    await getBff().reconcileProfile(profileId, idempotencyKey, requestContext(event.locals));
    return { success: action === "sync" ? "Runtime sync requested." : "Runtime rebuild requested. Orchestrator will verify the matching image before building when required." };
  } catch (error) {
    const mapped = safeError(error, event.locals.requestId);
    return fail(mapped.status, { error: mapped });
  }
}

export const actions: Actions = {
  sync: (event) => reconcile(event, "sync"),
  rebuild: (event) => reconcile(event, "rebuild"),
};
