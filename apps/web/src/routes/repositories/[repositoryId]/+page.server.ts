import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError, text } from "$lib/server/page.js";

const RESOURCE_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;

export const load: PageServerLoad = async ({ locals, params, setHeaders }) => {
  setHeaders({ "cache-control": "no-store" });
  if (!RESOURCE_ID.test(params.repositoryId)) {
    return {certificates:[],registryConnections:[],runtimeRevisions:{}, repositoryConnection: undefined, runtimeProfiles: [], runtimeKeys: {}, requestId: locals.requestId, error: { status: 400, reason: "invalid_identifier", message: "The repository connection identifier is invalid.", requestId: locals.requestId, retryable: false } };
  }
  const context = requestContext(locals);
  const [repository, profiles, connections, certificates] = await Promise.allSettled([
    getBff().getRepositoryConnection(params.repositoryId, context),
    getBff().listRuntimeProfiles(params.repositoryId, { pageSize: 100 }, context),
    getBff().listConnections(context),
    getBff().listCertificates(context),
  ]);
  const failure = repository.status === "rejected" ? repository.reason : profiles.status === "rejected" ? profiles.reason : undefined;
  return {
    runtimeRevisions: profiles.status === "fulfilled" ? Object.fromEntries(await Promise.all(profiles.value.runtimeProfiles.map(async profile => {
      try { return [profile.id, profile.activeRevisionId ? await getBff().getRuntimeRevision(profile.activeRevisionId,context) : undefined]; }
      catch { return [profile.id,undefined]; }
    }))) : {},
    certificates:certificates.status==="fulfilled" ? certificates.value.filter(c=>c.enabled):[],
    registryConnections:connections.status==="fulfilled" ? connections.value.filter(c=>c.kind==="ghcr" && c.status==="ready" && c.enabled):[],
    repositoryConnection: repository.status === "fulfilled" ? repository.value : undefined,
    runtimeProfiles: profiles.status === "fulfilled" ? profiles.value.runtimeProfiles : [],
    runtimeKeys: profiles.status === "fulfilled" ? Object.fromEntries(profiles.value.runtimeProfiles.map((profile) => [profile.id, actionKey()])) : {},
    analyzeKey: actionKey(),
    requestId: locals.requestId,
    ...(failure === undefined ? {} : { error: safeError(failure, locals.requestId) }),
  };
};

async function reconcile(event: Parameters<Actions[string]>[0], action: "sync" | "rebuild") {
  const form = await event.request.formData();
  const profileId = text(form, "profileId", 128);
  const idempotencyKey = text(form, "idempotencyKey", 36);
  if (!profileId || !idempotencyKey || !/^[0-9a-f-]{36}$/u.test(idempotencyKey)) return fail(400, { error: { message: "Refresh and try again." } });
  try {
    const profile=await getBff().getRuntimeProfile(profileId,requestContext(event.locals));
    if(profile.repositoryConnectionId!==event.params.repositoryId) return fail(400,{error:{message:"Choose a runtime belonging to this repository."}});
    const forceRebuild = action === "rebuild";
    await getBff().reconcileProfile(profileId, idempotencyKey, requestContext(event.locals), forceRebuild);
    return { success: forceRebuild ? "Runtime rebuild requested." : "Runtime sync requested." };
  } catch (error) {
    const mapped = safeError(error, event.locals.requestId);
    return fail(mapped.status, { error: mapped });
  }
}

export const actions: Actions = {
  analyze: async ({ request, params, locals }) => {
    const form = await request.formData();
    try {
      await getBff().analyzeRepository(params.repositoryId, text(form, "idempotencyKey", 128) ?? "", requestContext(locals));
      return { success: "Repository analysis requested." };
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return fail(mapped.status, { error: mapped });
    }
  },
  registry: async ({request,params,locals})=>{
    const f=await request.formData();
    try {
      const profileId=text(f,"profileId",128) ?? "";
      const profile=await getBff().getRuntimeProfile(profileId,requestContext(locals));
      if(profile.repositoryConnectionId!==params.repositoryId) return fail(400,{error:{message:"Choose a runtime belonging to this repository."}});
      await getBff().setRuntimeRegistry({runtimeProfileId:profileId,idempotencyKey:text(f,"idempotencyKey",36) ?? "",expectedResourceVersion:text(f,"resourceVersion",24) ?? "",registryConnectionId:text(f,"registryConnectionId",128) ?? ""},requestContext(locals));
      return {success:"Runtime image destination updated."};
    }catch(error){const mapped=safeError(error,locals.requestId);return fail(mapped.status,{error:{...mapped,...(mapped.status===409 ? {message:"This runtime changed. Close and reopen the dialog before saving again."} : {})}});}
  },
  certificate: async ({request,params,locals})=>{
    const f=await request.formData();
    try {
      const profileId=text(f,"profileId",128) ?? "";
      const profile=await getBff().getRuntimeProfile(profileId,requestContext(locals));
      if(profile.repositoryConnectionId!==params.repositoryId || profile.kind!=="ayni_managed") return fail(400,{error:{message:"Choose an Ayni runtime belonging to this repository."}});
      await getBff().setRuntimeCertificate({runtimeProfileId:profileId,idempotencyKey:text(f,"idempotencyKey",128) ?? "",expectedResourceVersion:text(f,"resourceVersion",24) ?? "",certificateId:text(f,"certificateId",128) ?? ""},requestContext(locals));
      return {success:"Runtime certificate updated. Public trust changes are proposed when needed."};
    }catch(error){const mapped=safeError(error,locals.requestId);return fail(mapped.status,{error:{...mapped,...(mapped.status===409 ? {message:"This runtime changed. Close and reopen the dialog before saving again."} : {})}});}
  },
  sync: (event) => reconcile(event, "sync"),
  rebuild: (event) => reconcile(event, "rebuild"),
};
