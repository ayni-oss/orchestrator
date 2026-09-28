import type { DevelopmentTaskDto, DevelopmentTaskPhaseDto, ProviderModelDto, RepositoryDto, RequestContext } from "@ayni-orchestrator/bff/server";
import { fail, redirect } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, isGitRevision, isSlug, lines, optionalLines, requestContext, safeError, text } from "$lib/server/page.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";

const PHASES = new Set<DevelopmentTaskPhaseDto>(["queued", "coding", "waiting_for_input", "validation_queued", "validating", "pr_queued", "preparing_pr", "completed", "failed", "cancelled", "unsupported", "unknown"]);

async function listRepositoryChoices(context: RequestContext): Promise<RepositoryDto[]> {
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

async function listModelChoices(context: RequestContext): Promise<{ provider?: string; models: ProviderModelDto[] }> {
  const models: ProviderModelDto[] = [];
  const seenTokens = new Set<string>();
  let provider: string | undefined;
  let pageToken: string | undefined;
  do {
    const page = await getBff().listProviderModels({ pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) }, context);
    provider ??= page.provider;
    models.push(...page.models);
    const nextPageToken = page.page.nextPageToken || undefined;
    if (nextPageToken === undefined) return { ...(provider === undefined ? {} : { provider }), models };
    if (seenTokens.has(nextPageToken)) throw new Error("Provider model pagination returned a repeated token.");
    seenTokens.add(nextPageToken);
    pageToken = nextPageToken;
  } while (true);
}

export const load: PageServerLoad = async ({ locals, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  const requestedPhase = url.searchParams.get("phase");
  const phase = requestedPhase !== null && PHASES.has(requestedPhase as DevelopmentTaskPhaseDto) ? requestedPhase as DevelopmentTaskPhaseDto : undefined;
  const requestedPageToken = url.searchParams.get("pageToken");
  const pageToken = requestedPageToken !== null && requestedPageToken.length <= 512 ? requestedPageToken : undefined;
  const context = requestContext(locals);
  const [tasks, repositories, models] = await Promise.allSettled([
    getBff().listDevelopmentTasks({ pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) }, phase, context),
    listRepositoryChoices(context),
    listModelChoices(context),
  ]);
  const failures = [tasks, repositories, models].filter((result): result is PromiseRejectedResult => result.status === "rejected");
  return {
    tasks: tasks.status === "fulfilled" ? tasks.value.tasks : [],
    nextPageToken: tasks.status === "fulfilled" ? tasks.value.page.nextPageToken || undefined : undefined,
    repositories: repositories.status === "fulfilled" ? repositories.value : [],
    provider: models.status === "fulfilled" ? models.value.provider : undefined,
    models: models.status === "fulfilled" ? models.value.models : [],
    phase,
    requestId: locals.requestId,
    idempotencyKey: actionKey(),
    backend: failures.length === 0 ? "connected" as const : failures.length === 3 ? "disconnected" as const : "degraded" as const,
    ...(failures[0] === undefined ? {} : { error: safeError(failures[0].reason, locals.requestId) }),
  };
};

export const actions: Actions = {
  create: async ({ locals, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    const repositoryId = text(form, "repositoryId", 64);
    const model = text(form, "model", 128);
    const startRevision = text(form, "startRevision", 40);
    const taskRequest = text(form, "request", 16_384);
    const criteriaValue = text(form, "acceptanceCriteria", 40_960);
    const criteria = criteriaValue === undefined ? undefined : lines(criteriaValue, 20, 2_048);
    const pathsValue = form.get("protectedPaths");
    const protectedPaths = optionalLines(typeof pathsValue === "string" ? pathsValue : undefined, 100, 512);
    const deadlineValue = typeof form.get("deadlineSeconds") === "string" ? String(form.get("deadlineSeconds")).trim() : "";
    const deadlineSeconds = deadlineValue === "" ? undefined : Number(deadlineValue);
    const pathsValid = protectedPaths !== undefined && new Set(protectedPaths).size === protectedPaths.length && protectedPaths.every((path) => !path.startsWith("/") && !path.endsWith("/") && !path.includes("\\") && path.split("/").every((part) => part !== "" && part !== "." && part !== ".."));
    if (idempotencyKey === undefined || !isCanonicalIdentifier(idempotencyKey) || repositoryId === undefined || !isSlug(repositoryId) || model === undefined || startRevision === undefined || !isGitRevision(startRevision) || taskRequest === undefined || criteria === undefined || !pathsValid || (deadlineSeconds !== undefined && (!Number.isInteger(deadlineSeconds) || deadlineSeconds < 1 || deadlineSeconds > 86_400))) {
      return fail(400, { action: "create", error: { status: 400, reason: "invalid_input", message: "Review the task fields, exact revision, criteria, constraints, and optional deadline.", requestId: locals.requestId, retryable: false } });
    }
    let task: DevelopmentTaskDto;
    try {
      task = await getBff().createDevelopmentTask({ idempotencyKey, input: { repositoryId, model, startRevision, request: taskRequest, acceptanceCriteria: criteria, changeConstraints: { protectedPaths, allowTestEdits: form.get("allowTestEdits") === "on" }, ...(deadlineSeconds === undefined ? {} : { deadlineSeconds }) } }, requestContext(locals));
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "create", error: mapped });
    }
    redirect(303, `/tasks/${task.id}`);
  },
};
