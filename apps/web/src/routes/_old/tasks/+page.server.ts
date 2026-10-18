import {redirect} from "@sveltejs/kit";
import type {PageServerLoad} from "./$types";
import {getBff} from "$lib/server/bff.js";
import {requestContext,safeError} from "$lib/server/page.js";
import {workspaceChoices,workspaceProjects} from "$lib/server/project-choices.js";
export const load:PageServerLoad=async({locals,setHeaders,url,depends})=>{
  depends("app:tasks","app:projects");setHeaders({"cache-control":"no-store"});
  const context=requestContext(locals),projectId=url.searchParams.get("project");
  if(projectId){
    const project=await getBff().getProject(projectId,context).catch(()=>undefined);
    if(project)redirect(303,`/_old/projects/${encodeURIComponent(project.id)}?directTask=1`);
  }
  const legacy=projectId!==null||url.searchParams.has("repository")||url.searchParams.has("profile")||url.searchParams.has("new");
  const [tasks,choices]=await Promise.allSettled([
    getBff().listDevelopmentTasks({pageSize:100,pageToken:url.searchParams.get("pageToken")??""},context),
    legacy?workspaceChoices(context).then(async workspaces=>(await Promise.all(workspaces.filter(w=>w.state==="active").map(w=>workspaceProjects(w.id,context)))).flat().filter(p=>p.state==="active")):Promise.resolve([]),
  ]);
  const phases:Record<string,string>={};
  if(tasks.status==="fulfilled")await Promise.allSettled(tasks.value.developmentTasks.map(async task=>{phases[task.id]=(await getBff().taskProgress(task.id,context)).attempts.at(-1)?.progress?.phase??"unavailable";}));
  const failed=[tasks,choices].find(value=>value.status==="rejected");
  return {tasks:tasks.status==="fulfilled"?tasks.value.developmentTasks:[],nextPageToken:tasks.status==="fulfilled"?tasks.value.page.nextPageToken:undefined,phases,
    projects:choices.status==="fulfilled"?choices.value:[],legacy,requestId:locals.requestId,backend:failed?"degraded":"connected",error:failed?.status==="rejected"?safeError(failed.reason,locals.requestId):undefined};
};
