import type { RuntimeProfileDto } from "@ayni-orchestrator/bff/server";
import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, isOwnerOrRepository, requestContext, safeError, text } from "$lib/server/page.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";

export const load: PageServerLoad = async ({ locals, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  const requestedPageToken = url.searchParams.get("pageToken");
  const pageToken = requestedPageToken !== null && requestedPageToken.length <= 512 ? requestedPageToken : undefined;
  const connectDialogOpen = url.searchParams.get("connect") === "1";
  const requestedAppName = url.searchParams.get("appName");
  const selectedInstallationId = url.searchParams.get("installationId") ?? "";
  try {
    const context = requestContext(locals);
    const bff = getBff();
    const [result, installations] = await Promise.all([
      bff.listRepositoryConnections({ pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) }, context),
      bff.listGithubInstallations(context),
    ]);
    const runtimeProfiles = await Promise.all(result.repositoryConnections.map(async (repository) => {
      try {
        const profiles = await bff.listRuntimeProfiles(repository.id, { pageSize: 10 }, context);
        return [repository.id, profiles.runtimeProfiles] as const;
      } catch {
        return [repository.id, []] as const;
      }
    }));
    const selectedAppName = installations.find(item => item.appName === requestedAppName)?.appName ?? installations[0]?.appName ?? "";
    const appInstallations = installations.filter(item => item.appName === selectedAppName);
    const installation = appInstallations.find((item) => item.installationId === selectedInstallationId) ?? appInstallations[0];
    const repositories = installation ? await bff.listGithubInstallationRepositories(installation.installationId, context) : [];
    return {
      repositoryConnections: result.repositoryConnections,
      runtimeProfiles: Object.fromEntries(runtimeProfiles) as Record<string, RuntimeProfileDto[]>,
      nextPageToken: result.page.nextPageToken || undefined,
      installations,
      selectedAppName,
      connectDialogOpen,
      selectedInstallationId: installation?.installationId ?? "",
      repositories,
      requestId: locals.requestId,
      backend: "connected" as const,
      idempotencyKey: actionKey(),
    };
  } catch (error) {
    return { repositoryConnections: [], runtimeProfiles: {} as Record<string, RuntimeProfileDto[]>, installations: [], selectedAppName: "", connectDialogOpen, selectedInstallationId: "", repositories: [], requestId: locals.requestId, backend: "disconnected" as const, idempotencyKey: actionKey(), error: safeError(error, locals.requestId) };
  }
};

export const actions: Actions = {
  connect: async ({ locals, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    const installationId = text(form, "installationId", 24) ?? "";
    const owner = text(form, "owner", 100) ?? "";
    const repository = text(form, "repository", 100) ?? "";
    const registryDestination = text(form, "registryDestination", 128);
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
      const connection = await bff.connectGithubRepository({idempotencyKey,owner,name:repository}, context);
      return { action: "connect", success: `Connected ${connection.displayName}.` };
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "connect", error: mapped });
    }
  },
};
