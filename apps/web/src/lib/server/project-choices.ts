import type { RequestContext, RepositoryConnectionDto } from "@ayni-orchestrator/bff/server";
import { getBff } from "./bff.js";
export async function projectRepositoryChoices(context: RequestContext): Promise<RepositoryConnectionDto[]> {
  const repositories: RepositoryConnectionDto[] = [];
  const seen = new Set<string>();
  let pageToken = "";
  do {
    const page = await getBff().listRepositoryConnections({ pageSize: 100, pageToken }, context);
    repositories.push(...page.repositoryConnections);
    pageToken = page.page.nextPageToken;
    if (pageToken && (seen.has(pageToken) || repositories.length >= 1000)) throw new Error("Repository choices could not be loaded completely.");
    seen.add(pageToken);
  } while (pageToken);
  return repositories;
}
