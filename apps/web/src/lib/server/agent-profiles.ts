import {fail} from "@sveltejs/kit";
import type {RequestEvent} from "@sveltejs/kit";
import type {AgentProfileSettingsDto} from "@ayni-orchestrator/bff/server";
import {profileTools,type ProfileRole} from "$lib/agent-profiles.js";
import {getBff} from "./bff.js";
import {requestContext,safeError,text} from "./page.js";
import {isCanonicalIdentifier} from "./validation.js";
export function parseProfile(form:FormData,role:ProfileRole):AgentProfileSettingsDto|undefined {
  if(form.get(`${role}.inherit`)==="true")return undefined;
  const maxModelCalls=Number(form.get(`${role}.calls`)),deadlineSeconds=String(form.get(`${role}.deadline`)??"");
  const allowedTools=form.getAll(`${role}.tools`).map(String),repositoryIds=form.getAll(`${role}.repositories`).map(String);
  const instructions=String(form.get(`${role}.instructions`)??"");
  if(!Number.isInteger(maxModelCalls)||maxModelCalls<1||maxModelCalls>128||!/^\d{1,5}$/u.test(deadlineSeconds)||Number(deadlineSeconds)<1||Number(deadlineSeconds)>86400||!allowedTools.length||allowedTools.some(t=>!profileTools[role].includes(t))||instructions.length>8192||repositoryIds.length>100)throw new Error("Invalid capability selection");
  const skills=[];
  for(let index=0;index<8;index++){
    const id=String(form.get(`${role}.skill.${index}.id`)??""),revision=String(form.get(`${role}.skill.${index}.revision`)??""),content=String(form.get(`${role}.skill.${index}.instructions`)??"");
    if(!id&&!revision&&!content)continue;
    if(!/^[a-zA-Z0-9._:/-]{1,128}$/u.test(id)||!/^[a-zA-Z0-9._:/-]{1,128}$/u.test(revision)||content.length>8192)throw new Error("Invalid pinned skill");
    skills.push({id,revision,instructions:content});
  }
  return {instructions,skills,allowedTools,repositoryIds,maxModelCalls,deadlineSeconds};
}
export async function saveAgentProfiles(event:RequestEvent,scope:"workspace"|"project") {
  const {locals,params,request}=event,form=await request.formData();
  const idempotencyKey=text(form,"idempotencyKey",36),expectedResourceVersion=text(form,"resourceVersion",20);
  if(!idempotencyKey||!isCanonicalIdentifier(idempotencyKey)||!expectedResourceVersion)return fail(400,{error:{message:"Reload agent profiles and try again.",requestId:locals.requestId}});
  let plannerProfile,workerProfile;
  try{plannerProfile=parseProfile(form,"planner");workerProfile=parseProfile(form,"worker");}catch{return fail(400,{error:{message:"Select allowed tools, valid budgets, and a revision for every skill.",requestId:locals.requestId}});}
  try{
    const bff=getBff(),ctx=requestContext(locals);
    const current=scope==="workspace"?await bff.getWorkspace(params.workspaceId!,ctx):await bff.getProject(params.projectId!,ctx);
    const agentDefaults={projectConnectionId:"",projectModel:"",codingConnectionId:"",codingModel:"",...current.agentDefaults,plannerProfile,workerProfile};
    if(scope==="workspace")await bff.updateWorkspace({workspaceId:current.id,idempotencyKey,expectedResourceVersion,displayName:current.displayName,agentDefaults},ctx);
    else await bff.updateProject({projectId:current.id,idempotencyKey,expectedResourceVersion,displayName:current.displayName,agentDefaults},ctx);
    return {success:"Agent profiles saved. New executions freeze these selections; admitted work keeps its profile."};
  }catch(error){const mapped=safeError(error,locals.requestId);return fail(mapped.status,{error:mapped});}
}
