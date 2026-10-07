import { json } from "@sveltejs/kit";
import type { RequestHandler } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { requestContext, safeError } from "$lib/server/page.js";
import type { RepositoryOptions } from "$lib/repository-options.js";

// Provider discovery is requested by the connection dialog, never by page load.
export const GET: RequestHandler = async ({ locals, url }) => {
  const headers = { "cache-control": "no-store" };
  try {
    const context = requestContext(locals);
    const bff = getBff();
    const installations = await bff.listGithubInstallations(context);
    const selectedAppName = installations.find(item => item.appName === url.searchParams.get("appName"))?.appName ?? installations[0]?.appName ?? "";
    const appInstallations = installations.filter(item => item.appName === selectedAppName);
    const installation = appInstallations.find(item => item.installationId === url.searchParams.get("installationId")) ?? appInstallations[0];
    const repositories = installation ? await bff.listGithubInstallationRepositories(installation.installationId, context) : [];
    return json({ installations, selectedAppName, selectedInstallationId: installation?.installationId ?? "", repositories } satisfies RepositoryOptions, { headers });
  } catch (error) {
    const mapped = safeError(error, locals.requestId);
    return json({ error: mapped }, { status: mapped.status, headers });
  }
};
