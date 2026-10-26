import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { shellSelection } from "$lib/server/shell-selection.js";
import { actionKey,requestContext,safeError,text } from "$lib/server/page.js";
export const load: PageServerLoad=async({locals,params,depends,setHeaders})=>{
  depends("app:runtimes");
  setHeaders({"cache-control":"no-store"});
  const keys={reconcile:actionKey()};
  const shell = await shellSelection(requestContext(locals), undefined, params.repositoryId);
  try {
    const context=requestContext(locals);
    const profile=await getBff().getRuntimeProfile(params.profileId,context);
    if (profile.repositoryConnectionId !== params.repositoryId) return { ...shell, keys, error: { status: 404, reason: "resource_not_found", message: "This runtime does not belong to this repository.", requestId: locals.requestId, retryable: false } };
    const [repository,revision,artifacts]=await Promise.all([
      getBff().getRepositoryConnection(profile.repositoryConnectionId,context),
      profile.activeRevisionId ? getBff().getRuntimeRevision(profile.activeRevisionId,context) : undefined,
      getBff().listRuntimeArtifacts(profile.id,{pageSize:100},context),
    ]);
    return {...shell,profile,repository,revision,artifacts:artifacts.runtimeArtifacts,keys};
  } catch(error) { return {...shell,keys,error:safeError(error,locals.requestId)}; }
};
async function reconcile(event:Parameters<Actions[string]>[0]) {
  const form=await event.request.formData();
  const key=text(form,"idempotencyKey",36);
  if(!key || !/^[0-9a-f-]{36}$/u.test(key)) return fail(400,{error:{message:"Refresh and try again."}});
  try {
    const bff=getBff(),context=requestContext(event.locals);
    const profile = await bff.getRuntimeProfile(event.params.profileId!, context);
    if (profile.repositoryConnectionId !== event.params.repositoryId) return fail(404, { error: { message: "This runtime does not belong to this repository." } });
    await bff.reconcileProfile(event.params.profileId!,key,context,form.get("forceRebuild")==="true");
    return {success:"Reconciliation requested."};
  } catch(error) { const mapped=safeError(error,event.locals.requestId); return fail(mapped.status,{error:mapped}); }
}
export const actions:Actions={reconcile};
