import {loadAgentConfiguration,loadAgentModels} from "$lib/server/agent-configuration.js";
import {saveAgentProfiles} from "$lib/server/agent-profiles.js";
import {fail} from "@sveltejs/kit";
import type {Actions,PageServerLoad} from "./$types";
import type {ProviderModelPageDto} from "@ayni-orchestrator/bff/server";
import {getBff} from "$lib/server/bff.js";
import {actionKey,requestContext,safeError,text} from "$lib/server/page.js";
import {isCanonicalIdentifier} from "$lib/server/validation.js";
import {projectRepositoryChoices,workspaceProjects} from "$lib/server/project-choices.js";
import {workspaceSchema} from "$lib/workspace-schema.js";
export const load:PageServerLoad=async({locals,params,depends,setHeaders})=>{
  depends("app:workspaces","app:projects","app:repositories","app:connections");setHeaders({"cache-control":"no-store"});
  const base={idempotencyKey:actionKey(),requestId:locals.requestId};
  try{
    const bff=getBff(),ctx=requestContext(locals),workspace=await bff.getWorkspace(params.workspaceId,ctx);
    const [repositories,projects,agentModels]=await Promise.all([projectRepositoryChoices(ctx),workspaceProjects(workspace.id,ctx),loadAgentModels(ctx).catch(()=>({connections:[],catalogues:{} as Record<string,ProviderModelPageDto>}))]);
    const configuration=await loadAgentConfiguration("workspace",workspace.id,ctx);
    return {...base,...configuration,workspace,repositories,projects,nextPageToken:"",connections:agentModels.connections,catalogues:agentModels.catalogues};
  }catch(error){return {...base,agentConfiguration:undefined,capabilities:undefined,configurationError:undefined,workspace:undefined,repositories:[],projects:[],nextPageToken:"",connections:[],catalogues:{} as Record<string,ProviderModelPageDto>,error:safeError(error,locals.requestId)};}
};
export const actions:Actions={
  agentProfiles:async event=>saveAgentProfiles(event,"workspace"),
  update:async event=>mutate(event,"update"),archive:async event=>mutate(event,"archive"),agentDefaults:async event=>mutate(event,"defaults"),
};
async function mutate({locals,params,request}:Parameters<NonNullable<Actions[string]>>[0],operation:"update"|"archive"|"defaults"){
  const form=await request.formData(),idempotencyKey=text(form,"idempotencyKey",36),expectedResourceVersion=text(form,"resourceVersion",20);
  if(!idempotencyKey||!isCanonicalIdentifier(idempotencyKey)||!expectedResourceVersion||!/^[1-9][0-9]{0,18}$/u.test(expectedResourceVersion))return fail(400,{error:{message:"Reload workspace settings and try again.",requestId:locals.requestId}});
  try{
    const bff=getBff(),ctx={...requestContext(locals),deadlineMs:30_000},input={idempotencyKey,expectedResourceVersion,workspaceId:params.workspaceId!};
    if(operation==="archive")await bff.archiveWorkspace(input,ctx);
    else if(operation==="update"){
      const parsed=workspaceSchema.safeParse({idempotencyKey,displayName:form.get("displayName"),repositoryConnectionIds:form.getAll("repositoryConnectionIds")});
      if(!parsed.success)return fail(400,{error:{message:"Enter a workspace name and select unique repositories.",requestId:locals.requestId}});
      const timeout=form.get("plannerIdleTimeoutSeconds");
      if(timeout!==null&&timeout!=="300"&&timeout!=="86400")return fail(400,{error:{message:"Choose a planner idle timeout of 5 minutes or 24 hours.",requestId:locals.requestId}});
      await bff.updateWorkspace({...input,...parsed.data,...(timeout===null?{}:{plannerIdleTimeoutSeconds:Number(timeout)})},ctx);
    }else{
      const fields=["projectConnectionId","projectModel","codingConnectionId","codingModel"] as const;
      const values=Object.fromEntries(fields.map(field=>[field,text(form,field,128)]));
      if(fields.some(field=>!values[field]))return fail(400,{error:{message:"Choose connections and available models for both agents.",requestId:locals.requestId}});
      const workspace=await bff.getWorkspace(input.workspaceId,ctx);
      await bff.updateWorkspace({...input,displayName:workspace.displayName,agentDefaults:{...workspace.agentDefaults,projectConnectionId:values.projectConnectionId!,projectModel:values.projectModel!,codingConnectionId:values.codingConnectionId!,codingModel:values.codingModel!}},ctx);
    }
    return {success:operation==="archive"?"Workspace archived. Project history is retained.":"Workspace settings updated."};
  }catch(error){const mapped=safeError(error,locals.requestId);return fail(mapped.status,{error:mapped});}
}
