import {fail,redirect} from "@sveltejs/kit";
import {superValidate,message} from "sveltekit-superforms";
import {zod} from "sveltekit-superforms/adapters";
import type {Actions,PageServerLoad} from "./$types";
import {getBff} from "$lib/server/bff.js";
import {actionKey,requestContext,safeError} from "$lib/server/page.js";
import {projectRepositoryChoices,workspaceProjectCounts} from "$lib/server/project-choices.js";
import {workspaceSchema} from "$lib/workspace-schema.js";
export const load:PageServerLoad=async({locals,depends,setHeaders,url})=>{
  depends("app:workspaces","app:repositories","app:projects");setHeaders({"cache-control":"no-store"});
  const context=requestContext(locals);
  const [workspaces,repositories,projectCounts]=await Promise.allSettled([getBff().listWorkspaces({pageSize:100,pageToken:url.searchParams.get("pageToken")??""},context),projectRepositoryChoices(context),workspaceProjectCounts(context)]);
  const failure=[workspaces,repositories,projectCounts].find(result=>result.status==="rejected");
  return {projectCounts:projectCounts.status==="fulfilled"?projectCounts.value:undefined,workspaces:workspaces.status==="fulfilled"?workspaces.value.workspaces:[],available:workspaces.status==="fulfilled",nextPageToken:workspaces.status==="fulfilled"?workspaces.value.page.nextPageToken:"",repositories:repositories.status==="fulfilled"?repositories.value:[],form:await superValidate({idempotencyKey:actionKey()},zod(workspaceSchema)),requestId:locals.requestId,...(failure?.status==="rejected"?{error:safeError(failure.reason,locals.requestId)}:{})};
};
export const actions:Actions={create:async({locals,request})=>{
  const form=await superValidate(request,zod(workspaceSchema));
  if(!form.valid)return fail(400,{form});
  let id:string;
  try{id=(await getBff().createWorkspace(form.data,requestContext(locals))).id;}
  catch(error){const mapped=safeError(error,locals.requestId);return message(form,mapped.message,{status:mapped.status as 400});}
  redirect(303,`/_old/workspaces/${encodeURIComponent(id)}`);
}};
