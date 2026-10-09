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
    const context = requestContext(locals);
    const bff = getBff();
    const [repositories, connections, builders] = await Promise.allSettled([
      bff.listRepositorySummaries({ pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) }, context),
      bff.listConnections(context),
      bff.listAyniBuilderVersions(context),
    ]);
    const registry = {
      builderVersions: builders.status === "fulfilled" ? builders.value : [],
      ...(builders.status === "rejected" ? { builderError: safeError(builders.reason, locals.requestId) } : {}),
      registryConnections: connections.status === "fulfilled" ? connections.value.filter(connection => connection.kind === "ghcr" && connection.status === "ready" && connection.enabled) : [],
      ...(connections.status === "rejected" ? { registryError: safeError(connections.reason, locals.requestId) } : {}),
    };
    if (repositories.status === "rejected") {
      return { ...registry, summaries: [], requestId: locals.requestId, backend: "disconnected" as const, idempotencyKey: actionKey(), error: safeError(repositories.reason, locals.requestId) };
    }
    return {
      ...registry,
      summaries: repositories.value.summaries,
      nextPageToken: repositories.value.page.nextPageToken || undefined,
      requestId: locals.requestId,
      backend: "connected" as const,
      idempotencyKey: actionKey(),
    };
  } catch (error) {
    return { builderVersions: [], registryConnections: [], summaries: [], requestId: locals.requestId, backend: "disconnected" as const, idempotencyKey: actionKey(), error: safeError(error, locals.requestId) };
  }
};

export const actions: Actions = {
  connect: async ({ locals, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    const installationId = text(form, "installationId", 24) ?? "";
    const owner = text(form, "owner", 100) ?? "";
    const repository = text(form, "repository", 100) ?? "";
    const useAyniChoice = form.get("useAyni");
    if (useAyniChoice !== null && useAyniChoice !== "true" && useAyniChoice !== "false") {
      return fail(400, { action: "connect", error: { message: "Choose whether to use Ayni.", requestId: locals.requestId } });
    }
    const useAyni = useAyniChoice !== "false";
    // Existing connection clients use the supported GHCR destination implicitly.
    const registryDestination = form.has("registryDestination") ? text(form, "registryDestination", 128) : "ghcr.io";
    if (registryDestination !== "ghcr.io") {
      return fail(400, { action: "connect", error: { status: 400, reason: "unsupported_registry", message: "Choose GitHub Container Registry as the destination.", requestId: locals.requestId, retryable: false } });
    }
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
      const registryConnectionId=useAyni ? text(form,"registryConnectionId",128) ?? "" : "";
      const ayniVersion = useAyni ? text(form, "ayniVersion", 128) ?? "" : "";
      if (ayniVersion && !(await bff.listAyniBuilderVersions(context)).includes(ayniVersion)) {
        return fail(400, { action: "connect", error: { message: "Choose an available Ayni builder version.", requestId: locals.requestId } });
      }
      const connection = await bff.connectGithubRepository({idempotencyKey,owner,name:repository,registryConnectionId,ayniVersion,...(useAyniChoice === null ? {} : {useAyni})}, context);
      return { action: "connect", success: `Connected ${connection.displayName}.` };
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "connect", error: mapped });
    }
  },
};
