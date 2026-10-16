import {loadAgentConfiguration,loadAgentModels} from "$lib/server/agent-configuration.js";
import {saveAgentProfiles} from "$lib/server/agent-profiles.js";
import {superValidate,message} from "sveltekit-superforms";
import {zod} from "sveltekit-superforms/adapters";
import {projectSchema} from "$lib/project-schema.js";
import {fail,redirect} from "@sveltejs/kit";
import type {Actions,PageServerLoad} from "./$types";
import type {ProviderModelPageDto} from "@ayni-orchestrator/bff/server";
import {getBff} from "$lib/server/bff.js";
import {actionKey,requestContext,safeError,text} from "$lib/server/page.js";
import {isCanonicalIdentifier} from "$lib/server/validation.js";
import {repositorySummaryChoices,workspaceProjects} from "$lib/server/project-choices.js";
import {workspaceSchema} from "$lib/workspace-schema.js";
export const load:PageServerLoad=async({locals,params,depends,setHeaders})=>{
  depends("app:workspaces","app:projects","app:repositories","app:runtimes","app:connections");setHeaders({"cache-control":"no-store"});
  const base={idempotencyKey:actionKey(),requestId:locals.requestId,form:await superValidate({idempotencyKey:actionKey(),workspaceId:params.workspaceId},zod(projectSchema))};
  try{
    const bff=getBff(),ctx=requestContext(locals),workspace=await bff.getWorkspace(params.workspaceId,ctx);
    const [repositorySummaries,projects,agentModels]=await Promise.all([repositorySummaryChoices(ctx),workspaceProjects(workspace.id,ctx),loadAgentModels(ctx).catch(()=>({connections:[],catalogues:{} as Record<string,ProviderModelPageDto>}))]);
    const configuration=await loadAgentConfiguration("workspace",workspace.id,ctx);
    return {...base,...configuration,workspace,repositorySummaries:repositorySummaries.filter(summary=>workspace.repositoryConnectionIds.includes(summary.repository.id)),repositories:repositorySummaries.map(summary=>summary.repository),projects,nextPageToken:"",connections:agentModels.connections,catalogues:agentModels.catalogues};
  }catch(error){return {...base,agentConfiguration:undefined,capabilities:undefined,configurationError:undefined,workspace:undefined,repositorySummaries:[],repositories:[],projects:[],nextPageToken:"",connections:[],catalogues:{} as Record<string,ProviderModelPageDto>,error:safeError(error,locals.requestId)};}
};
export const actions:Actions={
  create: async ({ locals, request, params }) => {
    const form = await superValidate(request, zod(projectSchema));
    if (!form.valid || form.data.workspaceId !== params.workspaceId) return fail(400, {form});
    let id: string;
    try {
      const context=requestContext(locals);
      const workspace=await getBff().getWorkspace(form.data.workspaceId,context);
      if(workspace.state!=="active" || form.data.repositoryConnectionIds.some(id=>!workspace.repositoryConnectionIds.includes(id)))return message(form,"Choose repositories in an active workspace.",{status:400});
      id = (await getBff().createProject(form.data, context)).id;
    } catch (error) {
      const mapped = safeError(error, locals.requestId);
      return message(form, mapped.message, {status:mapped.status as 400});
    }
    redirect(303, `/projects/${encodeURIComponent(id)}`);
  },
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
