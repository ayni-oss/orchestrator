import type { RequestContext, RepositoryConnectionDto } from "@ayni-orchestrator/bff/server";
import { getBff } from "./bff.js";
export async function projectRepositoryChoices(context: RequestContext): Promise<RepositoryConnectionDto[]> {
  const repositories: RepositoryConnectionDto[] = [];
  const seen = new Set<string>();
  let pageToken = "";
  do {
    const page = await getBff().listRepositoryConnections({ pageSize: 100, pageToken }, context);
    repositories.push(...page.repositoryConnections);
    pageToken = page.page.nextPageToken;
    if (pageToken && (seen.has(pageToken) || repositories.length >= 1000)) throw new Error("Repository choices could not be loaded completely.");
    seen.add(pageToken);
  } while (pageToken);
  return repositories;
}

export async function workspaceChoices(context:RequestContext) {
  const workspaces:Awaited<ReturnType<ReturnType<typeof getBff>["listWorkspaces"]>>["workspaces"]=[];
  const seen=new Set<string>();
  let pageToken="";
  do {
    const page=await getBff().listWorkspaces({pageSize:100,pageToken},context);
    workspaces.push(...page.workspaces);pageToken=page.page.nextPageToken;
    if(pageToken&&(seen.has(pageToken)||workspaces.length>=1000))throw new Error("Workspace choices could not be loaded completely.");
    seen.add(pageToken);
  }while(pageToken);
  return workspaces;
}

export async function workspaceProjects(workspaceId:string,context:RequestContext) {
  const projects:Awaited<ReturnType<ReturnType<typeof getBff>["listProjects"]>>["projects"]=[];
  const seen=new Set<string>();let pageToken="",loaded=0;
  do {
    const page=await getBff().listProjects({pageSize:100,pageToken},context);
    projects.push(...page.projects.filter(project=>project.workspaceId===workspaceId));
    loaded+=page.projects.length;pageToken=page.page.nextPageToken;
    if(pageToken&&(seen.has(pageToken)||loaded>=10000))throw new Error("Workspace projects could not be loaded completely.");
    seen.add(pageToken);
  }while(pageToken);
  return projects;
}
