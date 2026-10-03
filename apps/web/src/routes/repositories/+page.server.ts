import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, isOwnerOrRepository, requestContext, safeError, text } from "$lib/server/page.js";
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
    const owner = text(form, "owner", 100);
    const repository = text(form, "repository", 100);
    if (!idempotencyKey || !isCanonicalIdentifier(idempotencyKey) || !owner || !isOwnerOrRepository(owner) || !repository || !isOwnerOrRepository(repository)) {
      return fail(400, { action: "connect", error: { status:400, reason:"invalid_input", message:"Enter the GitHub owner and repository name.", requestId:locals.requestId, retryable:false } });
    }
    try {
      const connection = await getBff().connectGithubRepository({idempotencyKey,owner,name:repository}, requestContext(locals));
      return { action: "connect", success: `Connected ${connection.displayName}.` };
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "connect", error: mapped });
    }
  },
};
