import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, isOwnerOrRepository, requestContext, safeError, text } from "$lib/server/page.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";

export const load: PageServerLoad = async ({ locals, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  const requestedPageToken = url.searchParams.get("pageToken");
  const pageToken = requestedPageToken !== null && requestedPageToken.length <= 512 ? requestedPageToken : undefined;
  const selectedInstallationId = url.searchParams.get("installationId") ?? "";
  try {
    const context = requestContext(locals);
    const bff = getBff();
    const [result, installations] = await Promise.all([
      bff.listRepositoryConnections({ pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) }, context),
      bff.listGithubInstallations(context),
    ]);
    const installation = installations.find((item) => item.installationId === selectedInstallationId) ?? installations[0];
    const repositories = installation ? await bff.listGithubInstallationRepositories(installation.installationId, context) : [];
    return {
      repositoryConnections: result.repositoryConnections,
      nextPageToken: result.page.nextPageToken || undefined,
      installations,
      selectedInstallationId: installation?.installationId ?? "",
      repositories,
      requestId: locals.requestId,
      backend: "connected" as const,
      idempotencyKey: actionKey(),
    };
  } catch (error) {
    return { repositoryConnections: [], installations: [], selectedInstallationId: "", repositories: [], requestId: locals.requestId, backend: "disconnected" as const, idempotencyKey: actionKey(), error: safeError(error, locals.requestId) };
  }
};

export const actions: Actions = {
  connect: async ({ locals, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    const installationId = text(form, "installationId", 24) ?? "";
    const owner = text(form, "owner", 100) ?? "";
    const repository = text(form, "repository", 100) ?? "";
    if (!idempotencyKey || !isCanonicalIdentifier(idempotencyKey) || !/^\d{1,20}$/u.test(installationId) || !owner || !isOwnerOrRepository(owner) || !repository || !isOwnerOrRepository(repository)) {
      return fail(400, { action: "connect", error: { status:400, reason:"invalid_input", message:"Choose a GitHub owner and repository.", requestId:locals.requestId, retryable:false } });
    }
    try {
      const bff = getBff();
      const context = requestContext(locals);
      const installations = await bff.listGithubInstallations(context);
      if (!installations.some((item) => item.installationId === installationId && item.owner.toLowerCase() === owner.toLowerCase())) {
        return fail(400, { action: "connect", error: { status:400, reason:"invalid_selection", message:"That GitHub owner is no longer available to the selected App.", requestId:locals.requestId, retryable:false } });
      }
      const repositories = await bff.listGithubInstallationRepositories(installationId, context);
      if (!repositories.some((item) => item.owner.toLowerCase() === owner.toLowerCase() && item.name.toLowerCase() === repository.toLowerCase())) {
        return fail(400, { action: "connect", error: { status:400, reason:"invalid_selection", message:"Choose a repository the selected GitHub App installation can access.", requestId:locals.requestId, retryable:false } });
      }
      const connection = await bff.connectGithubRepository({idempotencyKey,owner,name:repository}, context);
      return { action: "connect", success: `Connected ${connection.displayName}.` };
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "connect", error: mapped });
    }
  },
};
