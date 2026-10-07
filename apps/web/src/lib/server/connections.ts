import { fail, type Actions, type RequestEvent } from "@sveltejs/kit";
import { BffError } from "@ayni-orchestrator/bff/server";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError } from "$lib/server/page.js";
export function connectionLoad(kind:string) {
  return async ({locals,setHeaders}:RequestEvent) => {
    setHeaders({"cache-control":"no-store"});
    try { return {connections:(await getBff().listConnections(requestContext(locals))).filter(c=>c.kind===kind),idempotencyKey:actionKey(),available:true}; }
    catch(error) {return {connections:[],idempotencyKey:actionKey(),available:false,error:safeError(error,locals.requestId)};}
  };
}
export function connectionActions(kind:string):Actions {
  const run = (operation:string):Actions[string] => async ({request,locals,setHeaders}) => {
    setHeaders({"cache-control":"no-store"});
    const f=await request.formData();
    const value=(key:string,max=256):string=>{
      const raw=f.get(key) ?? "";
      if(typeof raw!=="string" || raw.length>max) throw new BffError({status:400,reason:"invalid_input",message:"Connection input exceeds the supported bounds.",retryable:false});
      return raw;
    };
    try {
    const input={id:value("id"),idempotencyKey:value("idempotencyKey"),expectedResourceVersion:value("resourceVersion")};
    const credentials={privateKey:value("privateKey",32768),token:value("token",4096),pullToken:value("pullToken",4096)};
      const bff=getBff(),ctx=requestContext(locals);
      if(operation==="create") await bff.createConnection({kind,displayName:value("displayName"),appId:value("appId"),username:value("username"),imageNamespace:value("imageNamespace"),idempotencyKey:input.idempotencyKey,credentials},ctx);
      else if(operation==="replace") await bff.replaceConnectionCredentials({...input,credentials},ctx);
      else if(operation==="test") await bff.testConnection(input,ctx);
      else if(operation==="toggle") await bff.setConnectionEnabled({...input,enabled:value("enabled")==="true"},ctx);
      else if(operation==="delete") await bff.deleteConnection(input,ctx);
      return {success:"Connection updated."};
    } catch(error) {const mapped=safeError(error,locals.requestId);return fail(mapped.status,{error:mapped});}
  };
  return {create:run("create"),replace:run("replace"),test:run("test"),toggle:run("toggle"),delete:run("delete")};
}
