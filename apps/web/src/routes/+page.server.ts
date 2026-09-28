import { BffError, type BrowserErrorDto, type DevelopmentTaskDto } from "@ayni-orchestrator/bff/server";
import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";

function safeError(error: unknown, requestId: string): BrowserErrorDto {
  if (error instanceof BffError) return error.toDto();
  return {
    status: 500,
    reason: "internal",
    message: "the browser server could not complete the request",
    requestId,
    retryable: false,
  };
}

export const load: PageServerLoad = async ({ locals, setHeaders }) => {
  setHeaders({ "cache-control": "no-store" });
  const context = { requestId: locals.requestId, signal: locals.requestSignal };
  const [repositories, models, tasks] = await Promise.allSettled([
    getBff().listRepositories({ pageSize: 10 }, context),
    getBff().listProviderModels({ pageSize: 10 }, context),
    getBff().listDevelopmentTasks({ pageSize: 10 }, undefined, context),
  ]);
  const error = [repositories, models, tasks]
    .find((result): result is PromiseRejectedResult => result.status === "rejected");
  return {
    requestId: locals.requestId,
    backend: error === undefined ? "connected" as const : "degraded" as const,
    repositories: repositories.status === "fulfilled" ? repositories.value.repositories : [],
    provider: models.status === "fulfilled" ? models.value.provider : undefined,
    models: models.status === "fulfilled" ? models.value.models : [],
    tasks: tasks.status === "fulfilled" ? tasks.value.tasks : [],
    ...(error === undefined ? {} : { error: safeError(error.reason, locals.requestId) }),
  };
};

export const actions: Actions = {
  inspectTask: async ({ locals, request }) => {
    const form = await request.formData();
    const taskId = form.get("taskId");
    if (typeof taskId !== "string" || !isCanonicalIdentifier(taskId)) {
      return fail(400, {
        lookupError: {
          status: 400,
          reason: "invalid_identifier",
          message: "a valid task identifier is required",
          requestId: locals.requestId,
          retryable: false,
        } satisfies BrowserErrorDto,
      });
    }
    try {
      const task: DevelopmentTaskDto = await getBff().getDevelopmentTask(taskId, {
        requestId: locals.requestId,
        signal: locals.requestSignal,
      });
      return { inspectedTask: task };
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { lookupError: mapped });
    }
  },
};
