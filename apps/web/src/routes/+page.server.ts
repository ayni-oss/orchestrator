import type { RepositoryConnectionDto, RequestContext } from "@ayni-orchestrator/bff/server";
import type { PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { requestContext, safeError } from "$lib/server/page.js";

async function listAllRepositoryConnections(context: RequestContext): Promise<RepositoryConnectionDto[]> {
  const connections: RepositoryConnectionDto[] = [];
  const seenTokens = new Set<string>();
  let pageToken: string | undefined;
  do {
    const page = await getBff().listRepositoryConnections(
      { pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) },
      context,
    );
    connections.push(...page.repositoryConnections);
    const next = page.page.nextPageToken || undefined;
    if (next === undefined) return connections;
    if (seenTokens.has(next)) throw new Error("Repository pagination returned a repeated token.");
    seenTokens.add(next);
    pageToken = next;
  } while (true);
}

export const load: PageServerLoad = async ({ locals, setHeaders }) => {
  setHeaders({ "cache-control": "no-store" });
  const context = requestContext(locals);
  const [repositories, profiles, models, tasks] = await Promise.allSettled([
    listAllRepositoryConnections(context),
    getBff().listRuntimeProfiles(undefined, { pageSize: 100 }, context),
    getBff().listProviderModels({ pageSize: 100 }, context),
    getBff().listDevelopmentTasks({ pageSize: 20 }, context),
  ]);
  const failures = [repositories, profiles, models, tasks]
    .filter((result): result is PromiseRejectedResult => result.status === "rejected");
  return {
    requestId: locals.requestId,
    backend: failures.length === 0 ? "connected" as const : failures.length === 4 ? "disconnected" as const : "degraded" as const,
    repositoryConnections: repositories.status === "fulfilled" ? repositories.value : [],
    runtimeProfiles: profiles.status === "fulfilled" ? profiles.value.runtimeProfiles : [],
    provider: models.status === "fulfilled" ? models.value.provider : undefined,
    models: models.status === "fulfilled" ? models.value.models : [],
    tasks: tasks.status === "fulfilled" ? tasks.value.developmentTasks : [],
    ...(failures[0] === undefined ? {} : { error: safeError(failures[0].reason, locals.requestId) }),
  };
};
