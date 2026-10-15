import type {RequestContext} from "@ayni-orchestrator/bff/server";
import {getBff} from "./bff.js";
import {collectChoices} from "./choices.js";
export function projectRepositoryChoices(context:RequestContext){return collectChoices(async pageToken=>{const p=await getBff().listRepositoryConnections({pageSize:100,pageToken},context);return {items:p.repositoryConnections,nextPageToken:p.page.nextPageToken};});}
export function workspaceChoices(context:RequestContext){return collectChoices(async pageToken=>{const p=await getBff().listWorkspaces({pageSize:100,pageToken},context);return {items:p.workspaces,nextPageToken:p.page.nextPageToken};});}
export async function workspaceProjects(workspaceId:string,context:RequestContext){return (await collectChoices(async pageToken=>{const p=await getBff().listProjects({pageSize:100,pageToken},context);return {items:p.projects,nextPageToken:p.page.nextPageToken};},10000)).filter(p=>p.workspaceId===workspaceId);}
