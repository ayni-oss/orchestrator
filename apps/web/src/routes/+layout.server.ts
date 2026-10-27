import type { LayoutServerLoad } from './$types';
import type { Workspace, Project } from '$lib/workspace-types';
export const load: LayoutServerLoad = async ({ fetch, url }) => {
  const request = async (method: string, body: unknown) => {
    const response = await fetch(`/api/${method}`, {method:'POST',headers:{'Content-Type':'application/json','x-console-request':'1'},body:JSON.stringify(body)});
    if(!response.ok)throw new Error('The local service could not load this workspace.');
    return response.json();
  };
  try {
    const workspaces = await request('ListWorkspaces',{afterId:url.searchParams.get('afterWorkspace')||'',limit:50}) as {workspaces?:Workspace[];nextAfterId?:string};
    const selected = url.searchParams.get('workspace');
    let workspace = workspaces.workspaces?.find((item: {id:string})=>item.id===selected);
    // A newly acknowledged workspace may lie beyond the first page.
    let cursor=workspaces.nextAfterId;let scanned=0;
    while(selected && !workspace && cursor && scanned++<20){const result=await request('ListWorkspaces',{afterId:cursor,limit:100});workspace=result.workspaces?.find((item:{id:string})=>item.id===selected);cursor=result.nextAfterId;}
    const projects:{projects?:Project[];nextAfterId?:string}=workspace?await request('ListProjects',{workspaceId:workspace.id,afterId:url.searchParams.get('afterProject')||'',limit:50}):{};
    return {workspaces:workspaces.workspaces||[],workspace,projects:projects.projects||[],nextWorkspace:workspaces.nextAfterId||'',nextProject:projects.nextAfterId||'',error:''};
  }catch{return {workspaces:[],projects:[],workspace:undefined,nextWorkspace:'',nextProject:'',error:'The local service could not load workspaces. Retry when it is ready.'};}
};
