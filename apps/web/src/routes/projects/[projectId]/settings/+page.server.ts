import {saveAgentProfile} from "$lib/server/agent-profiles.js";
import {projectView} from "$lib/project-navigation.js";
import {loadAgentConfiguration,loadAgentModels} from "$lib/server/agent-configuration.js";
import {submitDirectTask} from "$lib/server/tasks.js";
import { fail, redirect } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError, text } from "$lib/server/page.js";
import { isResourceVersion, isCanonicalIdentifier } from "$lib/server/validation.js";
import { projectRepositoryChoices, workspaceChoices, workspaceProjects } from "$lib/server/project-choices.js";
import { projectSchema } from "$lib/project-schema.js";
import type { ProjectConversationDto, ProviderModelPageDto, RequestContext } from "@ayni-orchestrator/bff/server";

export const load: PageServerLoad = async ({ locals, depends, params, setHeaders, url }) => {
  if(url.searchParams.get("view") === "tasks") redirect(307,`/_old/projects/${encodeURIComponent(params.projectId)}${url.search}`);
  depends("app:workspaces", "app:projects", "app:runtimes", "app:tasks", "app:conversations", "app:connections");
  depends("app:repositories");
  setHeaders({ "cache-control": "no-store" });
  const workspaces = await workspaceChoices(requestContext(locals)).catch(() => []);
  const base = { workspaces, view: projectView(url.searchParams.get("view")), planVersion: url.searchParams.get("version") ?? "", idempotencyKey: actionKey(), taskKey: actionKey(), requestId: locals.requestId };
  try {
    const context = requestContext(locals), bff = getBff();
    const storedProject = await bff.getProject(params.projectId, context);
    const workspace = await bff.getWorkspace(storedProject.workspaceId,context);
    const projects = await workspaceProjects(workspace.id, context);
    const project = storedProject;
    const configuration=await loadAgentConfiguration("project",project.id,context);
    const [repositories, profiles, tasks, choices, agentModels, conversationResult] = await Promise.all([
      Promise.all(project.repositoryConnectionIds.map(id => bff.getRepositoryConnection(id, context))),
      Promise.all(project.repositoryConnectionIds.map(id => bff.listRuntimeProfiles(id, { pageSize: 100 }, context))),
      bff.listDevelopmentTasks({ pageSize: 100, pageToken: url.searchParams.get("pageToken") ?? "" }, context, project.id),
      projectRepositoryChoices(context).then(choices=>choices.filter(choice=>workspace.repositoryConnectionIds.includes(choice.id))),
      loadAgentModels(context).catch(()=>({connections:[],catalogues:{} as Record<string,ProviderModelPageDto>})),
      bff.listProjectConversations(project.id,context).then(conversations=>({conversations,error:undefined})).catch(error=>({conversations:[],error:safeError(error,locals.requestId)})),
    ]);
    const llmConnections=agentModels.connections,connectionModels=agentModels.catalogues;
    const codingCatalogue=connectionModels[configuration.agentConfiguration?.effective.codingConnectionId??""];
    const conversationId=url.searchParams.get("conversation") ?? (url.searchParams.has("new") ? undefined : conversationResult.conversations[0]?.id);
    const selectedConversation=conversationId ? conversationResult.conversations.find(c=>c.id===conversationId) : undefined;
    let conversation:ProjectConversationDto|undefined,conversationError=conversationResult.error;
    if(conversationId && !selectedConversation && !conversationError)conversationError={status:404,reason:"resource_not_found",message:"This conversation is no longer available in this project. Choose another conversation or start a new one.",requestId:locals.requestId,retryable:false};
    if(selectedConversation){
      try {const detail=await bff.getProjectConversation(selectedConversation.id,context);if(detail.projectId===project.id)conversation=detail;else conversationError={status:404,reason:"resource_not_found",message:"This conversation is no longer available in this project.",requestId:locals.requestId,retryable:false};}
      catch(error){conversationError=safeError(error,locals.requestId);}
    }
    const taskProgress:Record<string,Awaited<ReturnType<typeof bff.taskProgress>>>={};
    const taskIds=new Set([...tasks.developmentTasks.map(task=>task.id),...(conversation?.proposals.flatMap(proposal=>proposal.taskId?[proposal.taskId]:[])??[])]);
    await Promise.allSettled([...taskIds].map(async id=>{taskProgress[id]=await bff.taskProgress(id,context);}));
    return { ...base, projects, ...configuration, llmConnections, connectionModels, conversations:conversationResult.conversations, conversation, conversationError, taskProgress, project, workspace, repositories, profiles: profiles.flatMap(page => page.runtimeProfiles), tasks: tasks.developmentTasks, nextPageToken: tasks.page.nextPageToken, choices, models:codingCatalogue?.models ?? [], provider:codingCatalogue?.provider ?? "" };
  } catch (error) {
    return { ...base, projects: [], agentConfiguration:undefined,capabilities:undefined,configurationError:undefined,llmConnections:[],connectionModels:{} as Record<string,ProviderModelPageDto>,conversations:[],conversation:undefined,conversationError:undefined,taskProgress:{} as Record<string,Awaited<ReturnType<ReturnType<typeof getBff>["taskProgress"]>>>,project: undefined, workspace:undefined, repositories: [], profiles: [], tasks: [], nextPageToken: "", choices:[], models:[], provider:"", error: safeError(error, locals.requestId) };
  }
};
export const actions: Actions = {
  agentProfile:async event=>saveAgentProfile(event,"project"),
  plannerSettings: async event => conversationAction(event,"settings"),
  cancelPlannerTurn: async event => conversationAction(event,"cancel"),
  createConversation: async event => conversationAction(event,"create"),
  sendMessage: async event => conversationAction(event,"send"),
  reviseModel: async event => conversationAction(event,"model"),
  decideProposal: async event => conversationAction(event,"decide"),
  decidePlan: async event => conversationAction(event,"plan"),
  agentDefaults: async ({locals,params,request}) => {
    const form=await request.formData();
    const inherit=form.get("inherit")==="true";
    const fields=["projectConnectionId","projectModel","codingConnectionId","codingModel"] as const;
    const values=Object.fromEntries(fields.map(field=>[field,text(form,field,128)]));
    const idempotencyKey=text(form,"idempotencyKey",36), expectedResourceVersion=text(form,"resourceVersion",20);
    if(!idempotencyKey || !isCanonicalIdentifier(idempotencyKey) || !expectedResourceVersion || (!inherit&&fields.some(field=>!values[field])))return fail(400,{error:{message:"Choose a connection and an available model for both agents.",requestId:locals.requestId}});
    try {
      const bff=getBff(),ctx={...requestContext(locals),deadlineMs:30_000},project=await bff.getProject(params.projectId,ctx);
      await bff.updateProject({idempotencyKey,projectId:project.id,expectedResourceVersion,displayName:project.displayName,agentDefaults:inherit?{...project.agentDefaults,projectConnectionId:"",projectModel:"",codingConnectionId:"",codingModel:""}:{...project.agentDefaults,projectConnectionId:values.projectConnectionId!,projectModel:values.projectModel!,codingConnectionId:values.codingConnectionId!,codingModel:values.codingModel!}},ctx);
      return {success:"Agent defaults updated."};
    }catch(error){const mapped=safeError(error,locals.requestId);return fail(mapped.status,{error:mapped});}
  },
  update: async (event) => mutate(event, false),
  archive: async (event) => mutate(event, true),
  createTask:submitDirectTask,
};
async function mutate({ locals, params, request }: Parameters<NonNullable<Actions[string]>>[0], archive: boolean) {
  const form = await request.formData();
  const idempotencyKey = text(form, "idempotencyKey", 36);
  const expectedResourceVersion = text(form, "resourceVersion", 20);
  const parsed = projectSchema.safeParse({idempotencyKey,displayName:form.get("displayName"),workspaceId:form.get("workspaceId"),goal:form.get("goal"),repositoryConnectionIds:form.getAll("repositoryConnectionIds")});
  if (!idempotencyKey || !isCanonicalIdentifier(idempotencyKey) || !expectedResourceVersion || !/^[1-9][0-9]{0,18}$/u.test(expectedResourceVersion) || (!archive && !parsed.success)) return fail(400, { error: { message: "Enter a project name, select at least one repository, and reload if settings have changed.", requestId: locals.requestId } });
  try {
    const input = { idempotencyKey, projectId: params.projectId!, expectedResourceVersion };
    if (archive) await getBff().archiveProject(input, requestContext(locals));
    else if (parsed.success) {
      const context=requestContext(locals), bff=getBff();
      const project=await bff.getProject(params.projectId!,context),workspace=await bff.getWorkspace(project.workspaceId,context);
      if(parsed.data.workspaceId!==workspace.id || workspace.state!=="active" || parsed.data.repositoryConnectionIds.some(id=>!workspace.repositoryConnectionIds.includes(id)))return fail(400,{error:{message:"Choose repositories in this project's active workspace.",requestId:locals.requestId}});
      await bff.updateProject({ ...input, displayName: parsed.data.displayName, goal:parsed.data.goal, repositoryConnectionIds:parsed.data.repositoryConnectionIds }, context);
    }
    return { success: archive ? "Project archived. History is retained." : "Project updated." };
  } catch (error) {
    const mapped = safeError(error, locals.requestId);
    return fail(mapped.status, { error: mapped });
  }
}

type ConversationActionContext = {
  form:FormData;
  idempotencyKey:string;
  projectId:string;
  requestId:string;
  ctx:RequestContext;
};

async function conversationAction(event:Parameters<NonNullable<Actions[string]>>[0],operation:"create"|"send"|"decide"|"model"|"plan"|"settings"|"cancel") {
  const {locals,params,request}=event,form=await request.formData();
  const idempotencyKey=text(form,"idempotencyKey",128),conversationId=text(form,"conversationId",128);
  if(!idempotencyKey || (operation!=="create"&&!conversationId))return conversationInputError(locals.requestId,"Reload the conversation and try again.");
  const context:ConversationActionContext={form,idempotencyKey,projectId:params.projectId!,requestId:locals.requestId,ctx:{...requestContext(locals),deadlineMs:30_000}};
  try {
    if(operation==="create")return await createConversation(context);
    const conversation=await getBff().getProjectConversation(conversationId!,context.ctx);
    if(conversation.projectId!==context.projectId)return conversationInputError(locals.requestId,"Select a conversation in this project.");
    if(operation==="settings")return await updatePlannerSettings(context,conversation);
    if(operation==="cancel")return await cancelPlannerTurn(context,conversation);
    if(operation==="send")return await sendConversationMessage(context,conversation.id);
    if(operation==="plan")return await decideConversationPlan(context,conversation);
    if(operation==="model")return await reviseConversationModel(context,conversation);
    return await decideConversationProposal(context,conversation);
  }catch(error){
    if(error && typeof error==="object" && "status" in error && error.status===303)throw error;
    const mapped=safeError(error,locals.requestId);
    if((operation==="plan"||operation==="model")&&(mapped.status===409||mapped.status===412))mapped.message="This plan cannot be approved with its saved inputs. Reload and select the latest version. If its source or runtime has changed, request a revised plan before approving again.";
    return fail(mapped.status,{error:mapped});
  }
}

function conversationInputError(requestId:string,message:string){
  return fail(400,{error:{message,requestId}});
}

async function createConversation({form,idempotencyKey,projectId,requestId,ctx}:ConversationActionContext){
  const content=text(form,"content",16384);
  if(!content)return conversationInputError(requestId,"Enter a message of at most 16384 characters.");
  const title=Array.from(content.replace(/\s+/gu," ")).slice(0,30).join("");
  const bff=getBff(),project=await bff.getProject(projectId,ctx);
  if(!project.repositoryConnectionIds.length)return conversationInputError(requestId,"Add repositories in project settings before starting a conversation.");
  const conversation=await bff.createProjectConversation({idempotencyKey,projectId:project.id,title,repositoryConnectionIds:project.repositoryConnectionIds},ctx);
  await bff.sendProjectMessage({idempotencyKey:`${idempotencyKey}-message`,conversationId:conversation.id,content},ctx);
  redirect(303,`/projects/${encodeURIComponent(project.id)}?conversation=${encodeURIComponent(conversation.id)}`);
}

async function sendConversationMessage({form,idempotencyKey,requestId,ctx}:ConversationActionContext,conversationId:string){
  const content=text(form,"content",16384);
  if(!content)return conversationInputError(requestId,"Enter your message.");
  await getBff().sendProjectMessage({idempotencyKey,conversationId,content},ctx);
  return {success:"Message sent."};
}

async function reviseConversationModel({form,idempotencyKey,requestId,ctx}:ConversationActionContext,conversation:ProjectConversationDto){
  const proposalId=text(form,"proposalId",128),connectionId=text(form,"connectionId",128),model=text(form,"model",128);
  const proposal=conversation.proposals.find(p=>p.id===proposalId && p.action==="create_task" && p.state==="pending");
  if(!proposal||!connectionId||!model)return conversationInputError(requestId,"Choose an available coding connection and model for this pending task plan.");
  const bff=getBff(),catalog=await bff.listProviderModels({pageSize:100},ctx,connectionId);
  if(!catalog.models.some(m=>m.id===model))return conversationInputError(requestId,"The selected model is no longer available.");
  const planId=text(form,"planId",128),version=text(form,"version",20);
  if(!planId||!version||conversation.plan?.id!==planId||!conversation.plan.versions.some(v=>v.version===version&&v.proposalIds.includes(proposal.id)))return conversationInputError(requestId,"Reload the current plan before changing its coding model.");
  await bff.decideProjectPlan({idempotencyKey,conversationId:conversation.id,planId,version,decision:"revise"},ctx);
  await bff.sendProjectMessage({idempotencyKey:`${idempotencyKey}-message`,conversationId:conversation.id,content:`Revise task plan ${proposal.id} for repository ${proposal.request?.repositoryConnectionId}: use coding connection ${connectionId} and model ${model}. Preserve the other requirements and prepare a new proposal for approval.`},ctx);
  return {success:"Coding model revision requested. Review the new plan before approving."};
}

async function decideConversationProposal({form,idempotencyKey,requestId,ctx}:ConversationActionContext,conversation:ProjectConversationDto){
  const proposalId=text(form,"proposalId",128),decision=text(form,"decision",16);
  if(!proposalId||!["approve","reject","revise"].includes(decision??"")||!conversation.proposals.some(p=>p.id===proposalId))return conversationInputError(requestId,"Select a pending proposal and a decision.");
  await getBff().decideProjectProposal({idempotencyKey,conversationId:conversation.id,proposalId,decision:decision!},ctx);
  return {success:decision==="approve"?"Approved action submitted.":decision==="revise"?"Send a message with the revisions you need.":"Proposal rejected."};
}

async function decideConversationPlan({form,idempotencyKey,requestId,ctx}:ConversationActionContext,conversation:ProjectConversationDto){
  const planId=text(form,"planId",128),version=text(form,"version",20),decision=text(form,"decision",16);
  if(!planId||!version||!/^([1-9][0-9]{0,19})$/u.test(version)||!['approve','reject','revise'].includes(decision??'')||conversation.plan?.id!==planId||!conversation.plan.versions.some(v=>v.version===version))return conversationInputError(requestId,"Select a saved plan version and a decision.");
  await getBff().decideProjectPlan({idempotencyKey,conversationId:conversation.id,planId,version,decision:decision!},ctx);
  return {success:decision==='approve'?`Plan version ${version} approved. Tasks submitted.`:decision==='revise'?"Send a message with the revisions you need, then review the new version.":`Plan version ${version} rejected.`};
}

async function updatePlannerSettings({form,idempotencyKey,projectId,requestId,ctx}:ConversationActionContext,conversation:ProjectConversationDto){
  const selected=text(form,"plannerIdleTimeoutSeconds",16),expectedUpdateCursor=text(form,"expectedUpdateCursor",20);
  if(!expectedUpdateCursor||!isResourceVersion(expectedUpdateCursor)||!['inherit','300','86400'].includes(selected??''))return conversationInputError(requestId,"Choose the workspace default, 5 minutes, or 24 hours, then reload if settings have changed.");
  await getBff().updateProjectConversationSettings({idempotencyKey,projectId,conversationId:conversation.id,expectedUpdateCursor,inheritWorkspaceTimeout:selected==='inherit',...(selected==='inherit'?{}:{plannerIdleTimeoutSeconds:Number(selected)})},ctx);
  return {success:"Planner session settings updated."};
}
async function cancelPlannerTurn({form,idempotencyKey,projectId,requestId,ctx}:ConversationActionContext,conversation:ProjectConversationDto){
  const runId=text(form,"runId",128);
  if(!runId||!conversation.runs.some(run=>run.id===runId))return conversationInputError(requestId,"Select a planner turn in this conversation.");
  await getBff().cancelProjectAgentRun({idempotencyKey,projectId,conversationId:conversation.id,runId},ctx);
  return {success:"Planner turn cancellation requested. Saved history is retained."};
}
