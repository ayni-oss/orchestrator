import type { PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { requestContext, safeError } from "$lib/server/page.js";
import { registryConfiguration } from "$lib/connection.js";

export const load: PageServerLoad = async ({ locals, depends, setHeaders }) => {
  depends("app:connections");
  setHeaders({ "cache-control": "no-store" });
  try {
    const connections = await getBff().listConnections(requestContext(locals));
    return { registryConfiguration: registryConfiguration(connections) };
  } catch (error) {
    return { registryConfiguration: { status: "warning", description: "Container registry permission status is unavailable." }, error: safeError(error, locals.requestId) };
  }
};
