import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, isGitRevision, isSlug, requestContext, safeError, text } from "$lib/server/page.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";

export const load: PageServerLoad = async ({ locals, params, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  if (!isSlug(params.repositoryId)) return { repository: undefined, preparations: [], requestId: locals.requestId, idempotencyKey: actionKey(), error: { status: 400, reason: "invalid_identifier", message: "The repository identifier is invalid.", requestId: locals.requestId, retryable: false } };
  const requestedPageToken = url.searchParams.get("pageToken");
  const pageToken = requestedPageToken !== null && requestedPageToken.length <= 512 ? requestedPageToken : undefined;
  const context = requestContext(locals);
  const [repository, preparations] = await Promise.allSettled([
    getBff().getRepository(params.repositoryId, context),
    getBff().listPreparations(params.repositoryId, { pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) }, context),
  ]);
  const failure = repository.status === "rejected" ? repository.reason : preparations.status === "rejected" ? preparations.reason : undefined;
  return {
    repository: repository.status === "fulfilled" ? repository.value : undefined,
    preparations: preparations.status === "fulfilled" ? preparations.value.preparations : [],
    nextPageToken: preparations.status === "fulfilled" ? preparations.value.page.nextPageToken || undefined : undefined,
    requestId: locals.requestId,
    idempotencyKey: actionKey(),
    ...(failure === undefined ? {} : { error: safeError(failure, locals.requestId) }),
  };
};

export const actions: Actions = {
  prepare: async ({ locals, params, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    const revision = text(form, "revision", 40);
    if (idempotencyKey === undefined || !isCanonicalIdentifier(idempotencyKey) || !isSlug(params.repositoryId) || revision === undefined || !isGitRevision(revision)) {
      return fail(400, { action: "prepare", error: { status: 400, reason: "invalid_input", message: "Provide a full lowercase 40-character Git commit revision.", requestId: locals.requestId, retryable: false } });
    }
    try {
      const result = await getBff().prepareRepository({ idempotencyKey, repositoryId: params.repositoryId, revision }, requestContext(locals));
      return { action: "prepare", success: result.reused ? "Reused the compatible ready preparation." : "Repository preparation was queued.", preparation: result.preparation };
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "prepare", error: mapped });
    }
  },
};
