import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, isBranch, isOwnerOrRepository, requestContext, safeError, text } from "$lib/server/page.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";

export const load: PageServerLoad = async ({ locals, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  const requestedPageToken = url.searchParams.get("pageToken");
  const pageToken = requestedPageToken !== null && requestedPageToken.length <= 512 ? requestedPageToken : undefined;
  try {
    const result = await getBff().listRepositoryConnections(
      { pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) },
      requestContext(locals),
    );
    return {
      repositoryConnections: result.repositoryConnections,
      nextPageToken: result.page.nextPageToken || undefined,
      requestId: locals.requestId,
      backend: "connected" as const,
      idempotencyKey: actionKey(),
    };
  } catch (error) {
    return { repositoryConnections: [], requestId: locals.requestId, backend: "disconnected" as const, idempotencyKey: actionKey(), error: safeError(error, locals.requestId) };
  }
};

export const actions: Actions = {
  connect: async ({ locals, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    const repositoryId = text(form, "repositoryId", 20);
    const owner = text(form, "owner", 100);
    const repository = text(form, "repository", 100);
    const installationId = text(form, "installationId", 20);
    const defaultBranch = text(form, "defaultBranch", 255);
    const displayName = text(form, "displayName", 256);
    if (
      idempotencyKey === undefined || !isCanonicalIdentifier(idempotencyKey)
      || repositoryId === undefined || !/^[1-9][0-9]{0,19}$/u.test(repositoryId)
      || owner === undefined || !isOwnerOrRepository(owner)
      || repository === undefined || !isOwnerOrRepository(repository)
      || installationId === undefined || !/^[1-9][0-9]{0,19}$/u.test(installationId)
      || defaultBranch === undefined || !isBranch(defaultBranch)
      || displayName === undefined
    ) {
      return fail(400, { action: "connect", error: { status: 400, reason: "invalid_input", message: "Review the provider repository identity and connection fields.", requestId: locals.requestId, retryable: false } });
    }
    try {
      const connection = await getBff().createRepositoryConnection({
        idempotencyKey,
        provider: {
          kind: "github",
          repositoryId,
          owner,
          name: repository,
          installationId,
          htmlUrl: `https://github.com/${owner}/${repository}`,
        },
        displayName,
        defaultBranch,
      }, requestContext(locals));
      return { action: "connect", success: `Connected ${connection.displayName}.` };
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "connect", error: mapped });
    }
  },
};
