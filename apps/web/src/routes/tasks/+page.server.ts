import type { DevelopmentTaskDto, ProviderModelDto, ProjectDto, RequestContext, RuntimeProfileDto } from "@ayni-orchestrator/bff/server";
import { fail, redirect } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, lines, optionalLines, requestContext, safeError, text } from "$lib/server/page.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";

const RESOURCE_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;

async function listProjectChoices(context: RequestContext): Promise<ProjectDto[]> {
  const values: ProjectDto[] = [];
  let pageToken: string | undefined;
  do {
    const result = await getBff().listProjects({pageSize:100, ...(pageToken === undefined ? {} : {pageToken})}, context, true);
    values.push(...result.projects);
    pageToken = result.page.nextPageToken || undefined;
  } while (pageToken !== undefined && values.length < 1000);
  return values;
}

async function listProfileChoices(context: RequestContext): Promise<RuntimeProfileDto[]> {
  const values: RuntimeProfileDto[] = [];
  let pageToken: string | undefined;
  do {
    const page = await getBff().listRuntimeProfiles(undefined, { pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) }, context);
    values.push(...page.runtimeProfiles);
    pageToken = page.page.nextPageToken || undefined;
  } while (pageToken !== undefined && values.length < 1000);
  return values;
}

async function listModelChoices(context: RequestContext): Promise<{ provider?: string; models: ProviderModelDto[] }> {
  const models: ProviderModelDto[] = [];
  let provider: string | undefined;
  let pageToken: string | undefined;
  do {
    const page = await getBff().listProviderModels({ pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) }, context);
    provider ??= page.provider;
    models.push(...page.models);
    pageToken = page.page.nextPageToken || undefined;
  } while (pageToken !== undefined && models.length < 1000);
  return { ...(provider === undefined ? {} : { provider }), models };
}

export const load: PageServerLoad = async ({ locals, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  const token = url.searchParams.get("pageToken");
  const pageToken = token !== null && token.length <= 512 ? token : undefined;
  const context = requestContext(locals);
  const [tasks, repositories, profiles, models] = await Promise.allSettled([
    getBff().listDevelopmentTasks({ pageSize: 100, ...(pageToken === undefined ? {} : { pageToken }) }, context),
    listProjectChoices(context),
    listProfileChoices(context),
    listModelChoices(context),
  ]);
  const failures = [tasks, repositories, profiles, models].filter((result): result is PromiseRejectedResult => result.status === "rejected");
  return {
    tasks: tasks.status === "fulfilled" ? tasks.value.developmentTasks : [],
    nextPageToken: tasks.status === "fulfilled" ? tasks.value.page.nextPageToken || undefined : undefined,
    projects: repositories.status === "fulfilled" ? repositories.value : [],
    runtimeProfiles: profiles.status === "fulfilled" ? profiles.value : [],
    provider: models.status === "fulfilled" ? models.value.provider : undefined,
    models: models.status === "fulfilled" ? models.value.models : [],
    requestId: locals.requestId,
    idempotencyKey: actionKey(),
    backend: failures.length === 0 ? "connected" as const : failures.length === 4 ? "disconnected" as const : "degraded" as const,
    ...(failures[0] === undefined ? {} : { error: safeError(failures[0].reason, locals.requestId) }),
  };
};

async function taskSelection(projectId: string, repositoryConnectionId: string, runtimeProfileId: string, context: RequestContext) {
  const bff = getBff();
  const project = await bff.getProject(projectId, context);
  if (project.state !== "active") return undefined;
  if (!project.repositoryConnectionIds.includes(repositoryConnectionId)) return undefined;
  const profile = await bff.getRuntimeProfile(runtimeProfileId, context);
  if (profile.repositoryConnectionId !== repositoryConnectionId || profile.state !== "ready" || profile.support !== "operational" || !profile.activeRevisionId || !profile.activeRuntimeArtifactId) return undefined;
  const revision = await bff.getRuntimeRevision(profile.activeRevisionId, context);
  return {repositoryConnectionId, repositoryRevision: revision.repositoryRevision};
}

export const actions: Actions = {
  create: async ({ locals, request }) => {
    const form = await request.formData();
    const idempotencyKey = text(form, "idempotencyKey", 36);
    const projectId = text(form, "projectId", 128);
    const repositoryConnectionId = text(form, "repositoryConnectionId", 128);
    const runtimeProfileId = text(form, "runtimeProfileId", 128);
    const provider = text(form, "provider", 128);
    const model = text(form, "model", 128);

    const taskRequest = text(form, "request", 16_384);
    const criteriaValue = text(form, "acceptanceCriteria", 40_960);
    const criteria = criteriaValue === undefined ? undefined : lines(criteriaValue, 20, 2_048);
    const pathsValue = form.get("protectedPaths");
    const protectedPaths = optionalLines(typeof pathsValue === "string" ? pathsValue : undefined, 100, 512);
    const deadlineValue = typeof form.get("deadlineSeconds") === "string" ? String(form.get("deadlineSeconds")).trim() : "";
    const deadlineSeconds = deadlineValue === "" ? undefined : Number(deadlineValue);
    const pathsValid = protectedPaths !== undefined && new Set(protectedPaths).size === protectedPaths.length && protectedPaths.every((path) => !path.startsWith("/") && !path.endsWith("/") && !path.includes("\\") && path.split("/").every((part) => part !== "" && part !== "." && part !== ".."));
    if (
      idempotencyKey === undefined || !isCanonicalIdentifier(idempotencyKey)
      || projectId === undefined || !RESOURCE_ID.test(projectId)
      || repositoryConnectionId === undefined || !RESOURCE_ID.test(repositoryConnectionId)
      || runtimeProfileId === undefined || !RESOURCE_ID.test(runtimeProfileId)
      || provider === undefined || model === undefined
      || taskRequest === undefined || criteria === undefined || !pathsValid
      || (deadlineSeconds !== undefined && (!Number.isInteger(deadlineSeconds) || deadlineSeconds < 1 || deadlineSeconds > 86_400))
    ) {
      return fail(400, { action: "create", error: { status: 400, reason: "invalid_input", message: "Review the frozen repository, profile, agent/model, request, criteria, constraints, and deadline.", requestId: locals.requestId, retryable: false } });
    }
    let task: DevelopmentTaskDto;
    try {
      const context=requestContext(locals);
      const selection = await taskSelection(projectId, repositoryConnectionId, runtimeProfileId, context);
      if (!selection) return fail(409,{action:"create",error:{requestId:locals.requestId,message:"Select an active project with a ready runtime."}});
      const {repositoryRevision} = selection;
      task = await getBff().createDevelopmentTask({
        idempotencyKey,
        request: {
          projectId,
          repositoryConnectionId,
          repositoryRevision,
          runtimeProfileId,
          agent: { provider, agent: "coding-agent", model },
          request: taskRequest,
          publishDraft: true,
          acceptanceCriteria: criteria,
          changeConstraints: { protectedPaths, allowTestEdits: form.get("allowTestEdits") === "on" },
          ...(deadlineSeconds === undefined ? {} : { deadlineSeconds }),
        },
      }, requestContext(locals));
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "create", error: mapped });
    }
    redirect(303, `/tasks/${encodeURIComponent(task.id)}`);
  },
};
