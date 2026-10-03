import { fail, redirect } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, text, requestContext, safeError } from "$lib/server/page.js";

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
    idempotencyKey: actionKey(),
    requestId: locals.requestId,
    ...(failure === undefined ? {} : { error: safeError(failure, locals.requestId) }),
  };
};

export const actions: Actions = {
  inspect: async ({locals,params}) => {
    try { return { inspection: await getBff().inspectAyniRepository(params.repositoryId,requestContext(locals)) }; }
    catch(error) { const mapped=safeError(error,locals.requestId); return fail(mapped.status,{error:mapped}); }
  },
  onboard: async ({locals,params,request}) => {
    const form=await request.formData();
    const key=text(form,"idempotencyKey",36);
    if (!key || !/^[0-9a-f-]{36}$/u.test(key)) return fail(400,{error:{message:"Refresh and try again."}});
    let profile;
    try {
      const inspection=await getBff().inspectAyniRepository(params.repositoryId,requestContext(locals));
      if (!inspection.policyPresent || !inspection.lockCompatible) return fail(409,{error:{message:"Commit a supported Ayni policy and generated lock on the default branch, then inspect again."}});
      profile=await getBff().createRuntimeProfile({idempotencyKey:key,repositoryConnectionId:params.repositoryId,repositorySnapshotId:inspection.snapshotId,configuration:{kind:"ayni_managed",lockFingerprint:inspection.fingerprint,ayniVersion:inspection.version,workflowTemplateVersion:inspection.template,ociRepository:inspection.repository}},requestContext(locals));
      await getBff().installAutomation(profile.id,key,requestContext(locals));
    } catch(error) { const mapped=safeError(error,locals.requestId); return fail(mapped.status,{error:mapped}); }
    redirect(303,`/profiles/${encodeURIComponent(profile.id)}`);
  },
};
