import { fail, redirect } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError, text } from "$lib/server/page.js";
import { isCanonicalIdentifier } from "$lib/server/validation.js";
import { projectRepositoryChoices } from "$lib/server/project-choices.js";
import { projectSchema } from "$lib/project-schema.js";
import type { ProjectConversationDto, ProviderModelPageDto, RequestContext } from "@ayni-orchestrator/bff/server";
import { actions as taskActions } from "../../tasks/+page.server.js";
export const load: PageServerLoad = async ({ locals, depends, params, setHeaders, url }) => {
  depends("app:projects", "app:runtimes", "app:tasks", "app:conversations", "app:connections");
  depends("app:repositories");
  setHeaders({ "cache-control": "no-store" });
  const base = { idempotencyKey: actionKey(), taskKey: actionKey(), requestId: locals.requestId };
  try {
    const context = requestContext(locals), bff = getBff();
    const project = await bff.getProject(params.projectId, context);
    const [repositories, profiles, tasks, choices, models, allConnections, conversationResult] = await Promise.all([
      Promise.all(project.repositoryConnectionIds.map(id => bff.getRepositoryConnection(id, context))),
      Promise.all(project.repositoryConnectionIds.map(id => bff.listRuntimeProfiles(id, { pageSize: 100 }, context))),
      bff.listDevelopmentTasks({ pageSize: 100, pageToken: url.searchParams.get("pageToken") ?? "" }, context, project.id),
      projectRepositoryChoices(context),
      bff.listProviderModels({pageSize:100},context,project.agentDefaults?.codingConnectionId).catch(() => undefined),
      bff.listConnections(context).catch(() => undefined),
      bff.listProjectConversations(project.id,context).then(conversations=>({conversations,error:undefined})).catch(error=>({conversations:[],error:safeError(error,locals.requestId)})),
    ]);
    const llmConnections=allConnections?.filter(c=>c.kind==="openai" && c.enabled && c.status==="ready") ?? [];
    const connectionModels:Record<string,ProviderModelPageDto>={};
    await Promise.all(llmConnections.map(async c=>{try{connectionModels[c.id]=await bff.listProviderModels({pageSize:100},context,c.id);}catch{/* A missing catalogue disables model selection for this connection. */}}));
    const conversationId=url.searchParams.get("conversation");
    const selectedConversation=conversationId ? conversationResult.conversations.find(c=>c.id===conversationId) : conversationResult.conversations[0];
    let conversation:ProjectConversationDto|undefined,conversationError=conversationResult.error;
    if(selectedConversation){
      try {const detail=await bff.getProjectConversation(selectedConversation.id,context);if(detail.projectId===project.id)conversation=detail;else conversationError={status:404,reason:"resource_not_found",message:"This conversation is no longer available in this project.",requestId:locals.requestId,retryable:false};}
      catch(error){conversationError=safeError(error,locals.requestId);}
    }
    const taskProgress:Record<string,Awaited<ReturnType<typeof bff.taskProgress>>>={};
    await Promise.allSettled(tasks.developmentTasks.map(async task=>{taskProgress[task.id]=await bff.taskProgress(task.id,context);}));
    return { ...base, llmConnections, connectionModels, conversations:conversationResult.conversations, conversation, conversationError, taskProgress, project, repositories, profiles: profiles.flatMap(page => page.runtimeProfiles), tasks: tasks.developmentTasks, nextPageToken: tasks.page.nextPageToken, choices, models:models?.models ?? [], provider:models?.provider ?? "" };
  } catch (error) {
    return { ...base, llmConnections:[],connectionModels:{} as Record<string,ProviderModelPageDto>,conversations:[],conversation:undefined,conversationError:undefined,taskProgress:{} as Record<string,Awaited<ReturnType<ReturnType<typeof getBff>["taskProgress"]>>>,project: undefined, repositories: [], profiles: [], tasks: [], nextPageToken: "", choices:[], models:[], provider:"", error: safeError(error, locals.requestId) };
  }
};
export const actions: Actions = {
  createConversation: async event => conversationAction(event,"create"),
  sendMessage: async event => conversationAction(event,"send"),
  reviseModel: async event => conversationAction(event,"model"),
  decideProposal: async event => conversationAction(event,"decide"),
  agentDefaults: async ({locals,params,request}) => {
    const form=await request.formData();
    const fields=["projectConnectionId","projectModel","codingConnectionId","codingModel"] as const;
    const values=Object.fromEntries(fields.map(field=>[field,text(form,field,128)]));
    const idempotencyKey=text(form,"idempotencyKey",36), expectedResourceVersion=text(form,"resourceVersion",20);
    if(!idempotencyKey || !isCanonicalIdentifier(idempotencyKey) || !expectedResourceVersion || fields.some(field=>!values[field]))return fail(400,{error:{message:"Choose a connection and an available model for both agents.",requestId:locals.requestId}});
    try {
      const bff=getBff(),ctx={...requestContext(locals),deadlineMs:30_000},project=await bff.getProject(params.projectId,ctx);
      await bff.updateProject({idempotencyKey,projectId:project.id,expectedResourceVersion,displayName:project.displayName,agentDefaults:{projectConnectionId:values.projectConnectionId!,projectModel:values.projectModel!,codingConnectionId:values.codingConnectionId!,codingModel:values.codingModel!}},ctx);
      return {success:"Agent defaults updated."};
    }catch(error){const mapped=safeError(error,locals.requestId);return fail(mapped.status,{error:mapped});}
  },
  update: async (event) => mutate(event, false),
  archive: async (event) => mutate(event, true),
  createTask: async event => {
    const form = await event.request.clone().formData();
    if (form.get("projectId") !== event.params.projectId) return fail(400, {error:{message:"Select this project to submit work.",requestId:event.locals.requestId}});
    return taskActions.create!(event as never);
  },
};
async function mutate({ locals, params, request }: Parameters<NonNullable<Actions[string]>>[0], archive: boolean) {
  const form = await request.formData();
  const idempotencyKey = text(form, "idempotencyKey", 36);
  const expectedResourceVersion = text(form, "resourceVersion", 20);
  const parsed = projectSchema.safeParse({idempotencyKey,displayName:form.get("displayName"),repositoryConnectionIds:form.getAll("repositoryConnectionIds")});
  if (!idempotencyKey || !isCanonicalIdentifier(idempotencyKey) || !expectedResourceVersion || !/^[1-9][0-9]{0,18}$/u.test(expectedResourceVersion) || (!archive && !parsed.success)) return fail(400, { error: { message: "Enter a project name, select at least one repository, and reload if settings have changed.", requestId: locals.requestId } });
  try {
    const input = { idempotencyKey, projectId: params.projectId!, expectedResourceVersion };
    if (archive) await getBff().archiveProject(input, requestContext(locals));
    else if (parsed.success) await getBff().updateProject({ ...input, displayName: parsed.data.displayName, repositoryConnectionIds:parsed.data.repositoryConnectionIds }, requestContext(locals));
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

async function conversationAction(event:Parameters<NonNullable<Actions[string]>>[0],operation:"create"|"send"|"decide"|"model") {
  const {locals,params,request}=event,form=await request.formData();
  const idempotencyKey=text(form,"idempotencyKey",128),conversationId=text(form,"conversationId",128);
  if(!idempotencyKey || (operation!=="create"&&!conversationId))return conversationInputError(locals.requestId,"Reload the conversation and try again.");
  const context:ConversationActionContext={form,idempotencyKey,projectId:params.projectId!,requestId:locals.requestId,ctx:{...requestContext(locals),deadlineMs:30_000}};
  try {
    if(operation==="create")return await createConversation(context);
    const conversation=await getBff().getProjectConversation(conversationId!,context.ctx);
    if(conversation.projectId!==context.projectId)return conversationInputError(locals.requestId,"Select a conversation in this project.");
    if(operation==="send")return await sendConversationMessage(context,conversation.id);
    if(operation==="model")return await reviseConversationModel(context,conversation);
    return await decideConversationProposal(context,conversation);
  }catch(error){
    if(error && typeof error==="object" && "status" in error && error.status===303)throw error;
    const mapped=safeError(error,locals.requestId);
    return fail(mapped.status,{error:mapped});
  }
}

function conversationInputError(requestId:string,message:string){
  return fail(400,{error:{message,requestId}});
}

async function createConversation({form,idempotencyKey,projectId,requestId,ctx}:ConversationActionContext){
  const title=text(form,"title",256),ids=form.getAll("repositoryConnectionIds");
  const bff=getBff(),project=await bff.getProject(projectId,ctx);
  if(!title || !ids.length || ids.length>100 || ids.some(id=>typeof id!=="string"||!project.repositoryConnectionIds.includes(id)) || new Set(ids).size!==ids.length)return conversationInputError(requestId,"Enter a conversation title and choose project repositories.");
  const conversation=await bff.createProjectConversation({idempotencyKey,projectId:project.id,title,repositoryConnectionIds:ids as string[]},ctx);
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
  await bff.decideProjectProposal({idempotencyKey,conversationId:conversation.id,proposalId:proposal.id,decision:"revise"},ctx);
  await bff.sendProjectMessage({idempotencyKey:`${idempotencyKey}-message`,conversationId:conversation.id,content:`Revise task plan ${proposal.id} for repository ${proposal.request?.repositoryConnectionId}: use coding connection ${connectionId} and model ${model}. Preserve the other requirements and prepare a new proposal for approval.`},ctx);
  return {success:"Coding model revision requested. Review the new plan before approving."};
}

async function decideConversationProposal({form,idempotencyKey,requestId,ctx}:ConversationActionContext,conversation:ProjectConversationDto){
  const proposalId=text(form,"proposalId",128),decision=text(form,"decision",16);
  if(!proposalId||!["approve","reject","revise"].includes(decision??"")||!conversation.proposals.some(p=>p.id===proposalId))return conversationInputError(requestId,"Select a pending proposal and a decision.");
  await getBff().decideProjectProposal({idempotencyKey,conversationId:conversation.id,proposalId,decision:decision!},ctx);
  return {success:decision==="approve"?"Approved action submitted.":decision==="revise"?"Send a message with the revisions you need.":"Proposal rejected."};
}
