<script lang="ts">
  import AgentProfileSummary from "$lib/components/agent-profile-summary.svelte";
  import {tick} from "svelte";
  import WorkerUpdates from "$lib/components/worker-updates.svelte";
  import { projectHref } from "$lib/project-navigation.js";
  import { invalidate } from "$app/navigation";
  import type {ProjectViewData} from "$lib/project-view.js";
  import type { SubmitFunction } from "@sveltejs/kit";
  import { enhance } from "$lib/forms.js";
  import PlannerSession from "$lib/components/planner-session.svelte";
  import ConversationContext from "$lib/components/conversation-context.svelte";
  import ProjectProposal from "$lib/components/project-proposal.svelte";
  import ProjectPlanReview from "$lib/components/project-plan-review.svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { Textarea } from "$lib/components/ui/textarea/index.js";
  import { formatDate, words } from "$lib/presentation.js";
  let {data}:{data:ProjectViewData}=$props();
  let content=$state(""),pending=$state(false);
  const conversation=$derived(data.conversation);
  const active=$derived(data.project?.state==="active"&&data.workspace?.state==="active");
  const running=$derived(conversation?.runs.some(r=>r.state==="queued"||r.state==="running")??false);
  const submit:SubmitFunction=()=>{pending=true;return async({result,update})=>{try{if(result.type==="success")content="";await update({reset:false});}finally{pending=false;}};};
  async function discuss(update:NonNullable<ProjectViewData["conversation"]>["workerUpdates"][number]){
    content=`For task ${update.taskId}, attempt ${update.attemptId}, approved plan version ${update.origin.planVersion}:\nWorker question: ${update.requestedInput??update.result?.requestedInput??""}\nMy answer: `;
    await tick();
    const composer=document.getElementById("project-message");composer?.focus();composer?.scrollIntoView({block:"center",behavior:"smooth"});
  }
  function key(suffix:string){return `${data.idempotencyKey}-${suffix}`;}
  function name(id:string){return data.repositories.find(r=>r.id===id)?.displayName??id;}
</script>
<section class="space-y-4" aria-label="Project conversation">
  {#if data.conversationError}<Feedback kind="warning" title="Conversations unavailable" message={data.conversationError.message} /><Button variant="outline" onclick={()=>invalidate("app:conversations")}>Retry conversations</Button>{/if}
  {#if conversation}
    <div class="flex items-center justify-between gap-3"><h2 class="text-lg font-semibold">{conversation.title}</h2>{#if running}<StatusBadge status="running" pulse />{/if}</div>
    {#if data.view==="overview"}
    <Card.Root role="region" aria-label="Conversation"><Card.Content class="space-y-4">
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (The scroll region must support keyboard scrolling.) -->
      <div class="h-80 overflow-y-auto overscroll-contain space-y-4 rounded-md focus-visible:outline-2 focus-visible:outline-ring sm:h-96" role="region" aria-label="Conversation timeline" tabindex="0">
      <p class="text-sm text-muted-foreground">Researching {conversation.repositoryConnectionIds.map(name).join(", ")}. Tasks are submitted only after your approval.</p>
      {#key conversation.id}<PlannerSession {data} />{/key}
      {#key conversation.id}<ConversationContext projectId={conversation.projectId} conversationId={conversation.id} updateCursor={conversation.updateCursor} />{/key}
      <div class="space-y-4" aria-label="Conversation messages" aria-live="polite" aria-relevant="additions text">
        {#each conversation.messages as message (message.id)}
          <article class={['rounded-lg border p-4',message.role==='user'&&'bg-muted/40']}><div class="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground"><span>{message.role==="user"?"You":message.role==="assistant"?"Project agent":words(message.role)}</span><time datetime={message.createdAt}>{formatDate(message.createdAt)}</time></div><p class="whitespace-pre-wrap break-words text-sm">{message.content}</p></article>
        {:else}<p class="text-sm text-muted-foreground">Describe the change you want. The project agent will research these repositories and clarify the requirements.</p>{/each}
      </div>
      <WorkerUpdates updates={conversation.workerUpdates} onDiscuss={active?discuss:undefined} />
      {#if conversation.runs.length}<details class="text-sm"><summary class="cursor-pointer text-muted-foreground">Agent activity · {conversation.runs.length} runs</summary><div class="mt-3 space-y-2">{#each conversation.runs as run (run.id)}<InformationTable rows={[{label:"Run",value:words(run.state)},{label:"Model",value:run.model},{label:"Model calls",value:String(run.modelCalls)},{label:"Total tokens",value:run.usage?.totalTokens??"Unavailable"}]} /><AgentProfileSummary profile={run.plannerProfile} />{/each}</div></details>{/if}
      {#each conversation.runs.filter(r=>r.state==="failed"||r.state==="interrupted") as run (run.id)}<Feedback kind="warning" title={`Agent run ${words(run.state)}`} message={run.failure||"Send a message to explicitly retry. Previously approved tasks remain linked below."} />{/each}
      {#if running}<p role="status" class="text-sm text-muted-foreground">The agent is researching your request. You can return later; this conversation is saved.</p>{/if}
      </div>
      {#if active}<form method="POST" action="?/sendMessage" use:enhance={submit} class="space-y-3"><input type="hidden" name="idempotencyKey" value={key(`message-${conversation.id}-${conversation.updateCursor}`)} /><input type="hidden" name="conversationId" value={conversation.id} /><Label for="project-message">Message</Label><Textarea id="project-message" name="content" rows={3} required maxlength={16384} bind:value={content} placeholder="Describe the work, ask a question, or request a revision…" /><div class="flex flex-wrap justify-between gap-3"><p class="text-xs text-muted-foreground">To cancel or retry work, explicitly request the action here and approve its confirmation card.</p><Button type="submit" disabled={pending||running||!content.trim()||!data.agentConfiguration?.effective.projectModel||!data.agentConfiguration?.effective.codingModel}>{pending?"Sending…":"Send message"}</Button></div>{#if !data.agentConfiguration?.effective.projectModel||!data.agentConfiguration?.effective.codingModel}<p role="status" class="text-sm text-muted-foreground">Choose project and coding agent models in Project settings or workspace defaults before sending a message.</p>{/if}</form>{/if}
    </Card.Content></Card.Root>
    {/if}
    {#if conversation.plan}{#key `${conversation.plan.id}-${data.planVersion}`}<ProjectPlanReview {data} />{/key}{:else}<p class="text-sm text-muted-foreground">No plan yet. Discuss the project goal in <a class="underline" href={projectHref(data.project!.id,"overview",conversation.id)}>Overview</a> to prepare a plan for review.</p>{/if}
    {#if data.view==="overview"}{#each conversation.proposals.filter(p=>p.action!=="create_task"||!conversation.plan) as proposal (proposal.id)}
      <ProjectProposal {data} {proposal} allowDecision={proposal.action!=="create_task"} />
    {/each}{/if}
  {:else if !data.conversationError}<p class="text-sm text-muted-foreground">Start a conversation to research a project change and prepare tasks for approval.</p>{/if}
</section>
