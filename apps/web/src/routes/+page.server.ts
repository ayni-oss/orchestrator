import type { RepositoryDto, RequestContext } from "@ayni-orchestrator/bff/server";
import type { PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { requestContext, safeError } from "$lib/server/page.js";

async function listAllRepositories(context: RequestContext): Promise<RepositoryDto[]> {
  const repositories: RepositoryDto[] = [];
  const seenTokens = new Set<string>();
  let pageToken: string | undefined;
  do {
    const page = await getBff().listRepositories({ pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) }, context);
    repositories.push(...page.repositories);
    const nextPageToken = page.page.nextPageToken || undefined;
    if (nextPageToken === undefined) return repositories;
    if (seenTokens.has(nextPageToken)) throw new Error("Repository pagination returned a repeated token.");
    seenTokens.add(nextPageToken);
    pageToken = nextPageToken;
  } while (true);
}

export const load: PageServerLoad = async ({ locals, setHeaders }) => {
  setHeaders({ "cache-control": "no-store" });
  const context = requestContext(locals);
  const [repositories, models, tasks] = await Promise.allSettled([
    listAllRepositories(context),
    getBff().listProviderModels({ pageSize: 100 }, context),
    getBff().listDevelopmentTasks({ pageSize: 20 }, undefined, context),
  ]);
  const failures = [repositories, models, tasks].filter((result): result is PromiseRejectedResult => result.status === "rejected");
  return {
    requestId: locals.requestId,
    backend: failures.length === 0 ? "connected" as const : failures.length === 3 ? "disconnected" as const : "degraded" as const,
    repositories: repositories.status === "fulfilled" ? repositories.value : [],
    provider: models.status === "fulfilled" ? models.value.provider : undefined,
    models: models.status === "fulfilled" ? models.value.models : [],
    tasks: tasks.status === "fulfilled" ? tasks.value.tasks : [],
    ...(failures[0] === undefined ? {} : { error: safeError(failures[0].reason, locals.requestId) }),
  };
};
