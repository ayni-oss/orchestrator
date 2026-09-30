import type { PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { requestContext, safeError } from "$lib/server/page.js";

const RESOURCE_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;

export const load: PageServerLoad = async ({ locals, params, setHeaders }) => {
  setHeaders({ "cache-control": "no-store" });
  if (!RESOURCE_ID.test(params.repositoryId)) {
    return { repositoryConnection: undefined, runtimeProfiles: [], requestId: locals.requestId, error: { status: 400, reason: "invalid_identifier", message: "The repository connection identifier is invalid.", requestId: locals.requestId, retryable: false } };
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
    requestId: locals.requestId,
    ...(failure === undefined ? {} : { error: safeError(failure, locals.requestId) }),
  };
};
