import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey,requestContext,safeError,text } from "$lib/server/page.js";
export const load: PageServerLoad=async({locals,params,setHeaders})=>{
  setHeaders({"cache-control":"no-store"});
  const keys={install:actionKey(),reconcile:actionKey()};
  try {
    const context=requestContext(locals);
    const profile=await getBff().getRuntimeProfile(params.profileId,context);
    const [repository,revision,artifacts,automation]=await Promise.all([
      getBff().getRepositoryConnection(profile.repositoryConnectionId,context),
      profile.activeRevisionId ? getBff().getRuntimeRevision(profile.activeRevisionId,context) : undefined,
      getBff().listRuntimeArtifacts(profile.id,{pageSize:100},context),
      getBff().listAutomation(profile.id,context),
    ]);
    return {profile,repository,revision,artifacts:artifacts.runtimeArtifacts,installations:automation.installations,keys};
  } catch(error) { return {keys,error:safeError(error,locals.requestId)}; }
};
async function mutate(event:Parameters<Actions[string]>[0], install:boolean) {
  const form=await event.request.formData();
  const key=text(form,"idempotencyKey",36);
  if(!key || !/^[0-9a-f-]{36}$/u.test(key)) return fail(400,{error:{message:"Refresh and try again."}});
  try {
    const bff=getBff(),context=requestContext(event.locals);
    if(install) await bff.installAutomation(event.params.profileId!,key,context);
    else await bff.reconcileProfile(event.params.profileId!,key,context);
    return {success:install ? "Runtime setup requested. Matching pipelines will be reused." : "Reconciliation requested."};
  } catch(error) { const mapped=safeError(error,event.locals.requestId); return fail(mapped.status,{error:mapped}); }
}
export const actions:Actions={install:event=>mutate(event,true),reconcile:event=>mutate(event,false)};
