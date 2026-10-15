import type {DevelopmentTaskDto,RequestContext} from "@ayni-orchestrator/bff/server";
import type {RequestEvent} from "@sveltejs/kit";
import {fail,redirect} from "@sveltejs/kit";
import {loadAgentModels} from "./agent-configuration.js";
import {getBff} from "./bff.js";
import {lines,optionalLines,requestContext,safeError,text} from "./page.js";
import {isCanonicalIdentifier} from "./validation.js";
const RESOURCE_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;
async function taskSelection(projectId: string, repositoryConnectionId: string, runtimeProfileId: string, context: RequestContext) {
  const bff = getBff();
  const project = await bff.getProject(projectId, context);
  if (project.state !== "active") return undefined;
  const workspace=await bff.getWorkspace(project.workspaceId,context);
  if(workspace.state!=="active"||!workspace.repositoryConnectionIds.includes(repositoryConnectionId))return undefined;
  if (!project.repositoryConnectionIds.includes(repositoryConnectionId)) return undefined;
  const profile = await bff.getRuntimeProfile(runtimeProfileId, context);
  if (profile.repositoryConnectionId !== repositoryConnectionId || profile.state !== "ready" || profile.support !== "operational" || !profile.activeRevisionId || !profile.activeRuntimeArtifactId) return undefined;
  const revision = await bff.getRuntimeRevision(profile.activeRevisionId, context);
  return {repositoryConnectionId, repositoryRevision: revision.repositoryRevision};
}

function validProtectedPaths(protectedPaths: string[] | undefined): protectedPaths is string[] {
  return protectedPaths !== undefined && new Set(protectedPaths).size === protectedPaths.length && protectedPaths.every((path) => !path.startsWith("/") && !path.endsWith("/") && !path.includes("\\") && path.split("/").every((part) => part !== "" && part !== "." && part !== ".."));
}

function validDeadline(seconds: number | undefined): boolean {
  return seconds === undefined || (Number.isInteger(seconds) && seconds >= 1 && seconds <= 86_400);
}

function parseDirectTask(form:FormData) {
    const idempotencyKey = text(form, "idempotencyKey", 36);
    const projectId = text(form, "projectId", 128);
    const repositoryConnectionId = text(form, "repositoryConnectionId", 128);
    const runtimeProfileId = text(form, "runtimeProfileId", 128);
    const provider = text(form, "provider", 128);
    const model = text(form, "model", 128);
    const connectionId=text(form,"connectionId",128);

    const taskRequest = text(form, "request", 16_384);
    const criteriaValue = text(form, "acceptanceCriteria", 40_960);
    const criteria = criteriaValue === undefined ? undefined : lines(criteriaValue, 20, 2_048);
    const pathsValue = form.get("protectedPaths");
    const protectedPaths = optionalLines(typeof pathsValue === "string" ? pathsValue : undefined, 100, 512);
    const codingTurnsValue = form.get("codingTurns");
    const codingTurns = typeof codingTurnsValue === "string" && codingTurnsValue.trim() !== "" ? Number(codingTurnsValue) : 24;
    const deadlineValue = typeof form.get("deadlineSeconds") === "string" ? String(form.get("deadlineSeconds")).trim() : "";
    const deadlineSeconds = deadlineValue === "" ? undefined : Number(deadlineValue);

    if (
      idempotencyKey === undefined || !isCanonicalIdentifier(idempotencyKey)
      || projectId === undefined || !RESOURCE_ID.test(projectId)
      || repositoryConnectionId === undefined || !RESOURCE_ID.test(repositoryConnectionId)
      || runtimeProfileId === undefined || !RESOURCE_ID.test(runtimeProfileId)
      || provider === undefined || model === undefined
      || !Number.isInteger(codingTurns) || codingTurns < 1 || codingTurns > 128
      || taskRequest === undefined || criteria === undefined || !validProtectedPaths(protectedPaths)
      || !validDeadline(deadlineSeconds)
    ) {
      return undefined;
    }
    return {idempotencyKey,projectId,repositoryConnectionId,runtimeProfileId,provider,model,connectionId,taskRequest,criteria,protectedPaths,codingTurns,deadlineSeconds};
}

export async function submitDirectTask({locals,request,params}:RequestEvent) {
    const form=await request.formData(),parsed=parseDirectTask(form);
    if(!parsed)return fail(400,{action:"createTask",error:{status:400,reason:"invalid_input",message:"Review the frozen repository, profile, agent/model, request, criteria, constraints, and deadline.",requestId:locals.requestId,retryable:false}});
    const {idempotencyKey,projectId,repositoryConnectionId,runtimeProfileId,provider,model,connectionId,taskRequest,criteria,protectedPaths,codingTurns,deadlineSeconds}=parsed;

    if(projectId!==params.projectId)return fail(400,{action:"createTask",error:{message:"Select this project to submit work.",requestId:locals.requestId}});
    let task: DevelopmentTaskDto;
    try {
      const context=requestContext(locals);
      await getBff().getAgentCapabilities(context);
      await getBff().getProjectConfiguration(projectId,context);
      const {catalogues}=await loadAgentModels(context);
      if(!connectionId||!catalogues[connectionId]?.models.some(choice=>choice.id===model))return fail(409,{action:"createTask",error:{message:"Choose a model from an available coding connection.",requestId:locals.requestId}});
      const selection = await taskSelection(projectId, repositoryConnectionId, runtimeProfileId, context);
      if (!selection) return fail(409,{action:"createTask",error:{requestId:locals.requestId,message:"Select an active project with a ready runtime."}});
      const {repositoryRevision} = selection;
      task = await getBff().createDevelopmentTask({
        idempotencyKey,
        request: {
          projectId,
          ...(connectionId ? {connectionId} : {}),
          repositoryConnectionId,
          repositoryRevision,
          runtimeProfileId,
          agent: { provider, agent: "coding-agent", model },
          request: taskRequest,
          acceptanceCriteria: criteria,
          codingTurns,
          changeConstraints: { protectedPaths, allowTestEdits: form.get("allowTestEdits") === "on" },
          ...(deadlineSeconds === undefined ? {} : { deadlineSeconds }),
        },
      }, {...requestContext(locals),deadlineMs:30_000});
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { action: "createTask", error: mapped });
    }
    redirect(303, `/tasks/${encodeURIComponent(task.id)}`);
}
