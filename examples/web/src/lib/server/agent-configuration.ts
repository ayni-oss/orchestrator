import type {AgentConfigurationDto,RequestContext} from "@ayni-orchestrator/bff/server";
import {getBff} from "./bff.js";
import {safeError} from "./page.js";
import {collectChoices} from "./choices.js";
export async function loadAgentConfiguration(scope:"workspace"|"project",id:string,context:RequestContext){
  try {
    const bff=getBff(),capabilities=await bff.getAgentCapabilities(context);
    const application={projectConnectionId:"",projectModel:"",codingConnectionId:"",codingModel:"",plannerProfile:capabilities.roles.planner,workerProfile:capabilities.roles.worker};
    let agentConfiguration:AgentConfigurationDto;
    if(scope==="workspace"){
      const {workspace,effective}=await bff.getWorkspaceConfiguration(id,context);
      agentConfiguration={configured:workspace.agentDefaults,inherited:application,effective};
    }else{
      const {project,effective}=await bff.getProjectConfiguration(id,context);
      const workspace=await bff.getWorkspaceConfiguration(project.workspaceId,context);
      agentConfiguration={configured:project.agentDefaults,inherited:workspace.effective,effective};
    }
    return {capabilities,agentConfiguration,configurationError:undefined};
  }catch(error){return {capabilities:undefined,agentConfiguration:undefined,configurationError:safeError(error,context.requestId??"configuration","Agent configuration could not be loaded. Check the control plane and reload before submitting work.")};}
}
export async function loadAgentModels(context:RequestContext){
  const bff=getBff(),connections=(await bff.listConnections(context)).filter(c=>c.kind==="openai"&&c.enabled&&c.status==="ready");
  const catalogues:Record<string,Awaited<ReturnType<typeof bff.listProviderModels>>>={};
  await Promise.all(connections.map(async connection=>{try{
    let latest:Awaited<ReturnType<typeof bff.listProviderModels>>|undefined;
    const models=await collectChoices(async pageToken=>{latest=await bff.listProviderModels({pageSize:100,pageToken},context,connection.id);return {items:latest.models,nextPageToken:latest.page.nextPageToken};});
    if(latest)catalogues[connection.id]={...latest,models};
  }catch{/* An incomplete or unavailable catalogue cannot supply a selectable model. */}}));
  return {connections,catalogues};
}
