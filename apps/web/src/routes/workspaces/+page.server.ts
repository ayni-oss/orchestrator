import {fail,redirect} from "@sveltejs/kit";
import {superValidate,message} from "sveltekit-superforms";
import {zod} from "sveltekit-superforms/adapters";
import type {Actions,PageServerLoad} from "./$types";
import {getBff} from "$lib/server/bff.js";
import {actionKey,requestContext,safeError} from "$lib/server/page.js";
import {projectRepositoryChoices} from "$lib/server/project-choices.js";
import {workspaceSchema} from "$lib/workspace-schema.js";
export const load:PageServerLoad=async({locals,depends,setHeaders,url})=>{
  depends("app:workspaces","app:repositories");setHeaders({"cache-control":"no-store"});
  const context=requestContext(locals);
  const [workspaces,repositories]=await Promise.allSettled([getBff().listWorkspaces({pageSize:100,pageToken:url.searchParams.get("pageToken")??""},context),projectRepositoryChoices(context)]);
  const failure=[workspaces,repositories].find(result=>result.status==="rejected");
  return {workspaces:workspaces.status==="fulfilled"?workspaces.value.workspaces:[],available:workspaces.status==="fulfilled",nextPageToken:workspaces.status==="fulfilled"?workspaces.value.page.nextPageToken:"",repositories:repositories.status==="fulfilled"?repositories.value:[],form:await superValidate({idempotencyKey:actionKey()},zod(workspaceSchema)),requestId:locals.requestId,...(failure?.status==="rejected"?{error:safeError(failure.reason,locals.requestId)}:{})};
};
export const actions:Actions={create:async({locals,request})=>{
  const form=await superValidate(request,zod(workspaceSchema));
  if(!form.valid)return fail(400,{form});
  let id:string;
  try{id=(await getBff().createWorkspace(form.data,requestContext(locals))).id;}
  catch(error){const mapped=safeError(error,locals.requestId);return message(form,mapped.message,{status:mapped.status as 400});}
  redirect(303,`/workspaces/${encodeURIComponent(id)}`);
}};
