import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, isBranch, isOwnerOrRepository, isSha256, isSlug, requestContext, safeError, text } from "$lib/server/page.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";

export const load: PageServerLoad = async ({ locals, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  const requestedPageToken = url.searchParams.get("pageToken");
  const pageToken = requestedPageToken !== null && requestedPageToken.length <= 512 ? requestedPageToken : undefined;
  try {
    const result = await getBff().listRepositories({ pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) }, requestContext(locals));
    return { repositories: result.repositories, nextPageToken: result.page.nextPageToken || undefined, requestId: locals.requestId, backend: "connected" as const, idempotencyKey: actionKey() };
  } catch (error) {
    return { repositories: [], requestId: locals.requestId, backend: "disconnected" as const, idempotencyKey: actionKey(), error: safeError(error, locals.requestId) };
  }
};

export const actions: Actions = {
  enroll: async ({ locals, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    const id = text(form, "id", 64);
    const owner = text(form, "owner", 100);
    const repository = text(form, "repository", 100);
    const installationId = text(form, "installationId", 20);
    const baseBranch = text(form, "baseBranch", 255);
    const lockSha256 = text(form, "lockSha256", 64);
    if (idempotencyKey === undefined || !isCanonicalIdentifier(idempotencyKey) || id === undefined || !isSlug(id) || owner === undefined || !isOwnerOrRepository(owner) || repository === undefined || !isOwnerOrRepository(repository) || installationId === undefined || !/^[1-9][0-9]{0,18}$/u.test(installationId) || baseBranch === undefined || !isBranch(baseBranch) || lockSha256 === undefined || !isSha256(lockSha256)) {
      return fail(400, { action: "enroll", error: { status: 400, reason: "invalid_input", message: "Review the enrollment fields and provide a valid lock SHA-256 digest.", requestId: locals.requestId, retryable: false } });
    }
    try {
      const saved = await getBff().saveRepository({ idempotencyKey, repository: { id, owner, repository, installationId, baseBranch, lockSha256 } }, requestContext(locals));
      return { action: "enroll", success: `Enrolled ${saved.enrollment.owner}/${saved.enrollment.repository}.` };
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "enroll", error: mapped });
    }
  },
};
