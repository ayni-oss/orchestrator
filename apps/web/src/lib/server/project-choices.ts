import type {RequestContext} from "@ayni-orchestrator/bff/server";
import {getBff} from "./bff.js";
import {collectChoices} from "./choices.js";
export function projectRepositoryChoices(context:RequestContext){return collectChoices(async pageToken=>{const p=await getBff().listRepositoryConnections({pageSize:100,pageToken},context);return {items:p.repositoryConnections,nextPageToken:p.page.nextPageToken};});}
export function workspaceChoices(context:RequestContext){return collectChoices(async pageToken=>{const p=await getBff().listWorkspaces({pageSize:100,pageToken},context);return {items:p.workspaces,nextPageToken:p.page.nextPageToken};});}
export async function workspaceProjects(workspaceId:string,context:RequestContext){return (await collectChoices(async pageToken=>{const p=await getBff().listProjects({pageSize:100,pageToken},context);return {items:p.projects,nextPageToken:p.page.nextPageToken};},10000)).filter(p=>p.workspaceId===workspaceId);}

export async function workspaceProjectCounts(context:RequestContext){
  const projects=await collectChoices(async pageToken=>{const page=await getBff().listProjects({pageSize:100,pageToken},context);return {items:page.projects,nextPageToken:page.page.nextPageToken};},10000);
  const counts:Record<string,number>={};
  for(const project of projects)counts[project.workspaceId]=(counts[project.workspaceId]??0)+1;
  return counts;
}
