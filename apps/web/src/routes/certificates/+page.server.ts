import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { BffError } from "@ayni-orchestrator/bff/server";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError } from "$lib/server/page.js";
export const load: PageServerLoad = async ({locals,setHeaders}) => {
  setHeaders({"cache-control":"no-store"});
  try { return {certificates:await getBff().listCertificates(requestContext(locals)),idempotencyKey:actionKey(),available:true}; }
  catch(error) { return {certificates:[],idempotencyKey:actionKey(),available:false,error:safeError(error,locals.requestId)}; }
};
function mutation(operation:string):Actions[string] {
  return async ({request,locals,setHeaders}) => {
    setHeaders({"cache-control":"no-store"});
    const form=await request.formData();
    try {
      const value=(key:string,max=128)=>{
        const raw=form.get(key) ?? "";
        if(typeof raw!=="string" || raw.length>max) throw new BffError({status:400,reason:"invalid_input",message:"Certificate input exceeds the supported bounds.",retryable:false});
        return raw;
      };
      const input={id:value("id"),idempotencyKey:value("idempotencyKey"),expectedResourceVersion:value("resourceVersion",24)};
      const bff=getBff(),context=requestContext(locals);
      if(operation==="create" || operation==="replace") {
        const signingSeed=value("signingSeed",64),generate=value("mode")==="generate";
        if((generate && signingSeed!=="") || (!generate && !/^[0-9a-f]{64}$/u.test(signingSeed))) throw new BffError({status:400,reason:"invalid_input",message:"Import a 64-character lowercase hexadecimal signing seed or choose Generate.",retryable:false});
        const key={keyId:value("keyId"),signingSeed,generate};
        if(operation==="create") await bff.createCertificate({...key,displayName:value("displayName"),idempotencyKey:input.idempotencyKey},context);
        else await bff.replaceCertificate({...input,...key},context);
      } else if(operation==="rename") await bff.renameCertificate({...input,displayName:value("displayName")},context);
      else if(operation==="toggle") await bff.setCertificateEnabled({...input,enabled:value("enabled")==="true"},context);
      else if(operation==="default") await bff.setDefaultCertificate(input,context);
      else if(operation==="delete") await bff.deleteCertificate(input,context);
      return {success:"Certificate updated."};
    } catch(error) { const mapped=safeError(error,locals.requestId); return fail(mapped.status,{error:mapped}); }
    finally { form.delete("signingSeed"); }
  };
}
export const actions:Actions={create:mutation("create"),replace:mutation("replace"),rename:mutation("rename"),toggle:mutation("toggle"),makeDefault:mutation("default"),delete:mutation("delete")};
