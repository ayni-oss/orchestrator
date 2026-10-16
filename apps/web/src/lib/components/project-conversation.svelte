<script lang="ts">
  import {page} from "$app/state";
  import {untrack,onMount} from "svelte";
  import ConversationMessages from "$lib/components/conversation-messages.svelte";
  import { invalidate } from "$app/navigation";
  import type {ProjectViewData} from "$lib/project-view.js";
  import type { SubmitFunction } from "@sveltejs/kit";
  import { enhance } from "$lib/forms.js";
  import ProjectProposal from "$lib/components/project-proposal.svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { Textarea } from "$lib/components/ui/textarea/index.js";
  let {data}:{data:ProjectViewData}=$props();
  let content=$state(untrack(()=>{
    const update=data.conversation?.workerUpdates.find(update=>update.attemptId===page.url.searchParams.get("question"));
    return update?`For task ${update.taskId}, attempt ${update.attemptId}, approved plan version ${update.origin.planVersion}:\nWorker question: ${update.requestedInput??update.result?.requestedInput??""}\nMy answer: `:"";
  })),pending=$state(false);
  let composer:HTMLTextAreaElement|null=$state(null);
  onMount(()=>{if(page.url.searchParams.has("question"))composer?.focus();});
  const conversation=$derived(data.conversation);
  const active=$derived(data.project?.state==="active"&&data.workspace?.state==="active");
  const running=$derived(conversation?.runs.some(r=>r.state==="queued"||r.state==="running")??false);
  const submit:SubmitFunction=()=>{pending=true;return async({result,update})=>{try{if(result.type==="success")content="";await update({reset:false});}finally{pending=false;}};};
  function key(suffix:string){return `${data.idempotencyKey}-${suffix}`;}
</script>
<section class="flex min-h-0 flex-1 flex-col gap-4" aria-label="Project conversation">
  {#if data.conversationError}<Feedback kind="warning" title="Conversations unavailable" message={data.conversationError.message} /><Button variant="outline" onclick={()=>invalidate("app:conversations")}>Retry conversations</Button>{/if}
  {#if conversation}
    <div class="flex items-center justify-between gap-3"><h2 class="text-lg font-semibold">{conversation.title}</h2>{#if running}<StatusBadge status="running" pulse />{/if}</div>
    <Card.Root role="region" aria-label="Conversation" class="min-h-0 flex-1"><Card.Content class="flex min-h-0 flex-1 flex-col gap-4">
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (The scroll region must support keyboard scrolling.) -->
      <div class="min-h-32 flex-1 overflow-y-auto overscroll-contain space-y-4 rounded-md focus-visible:outline-2 focus-visible:outline-ring" role="region" aria-label="Conversation timeline" tabindex="0">
      {#key conversation.id}<ConversationMessages {conversation} />{/key}
      {#each conversation.runs.filter(r=>r.state==="failed"||r.state==="interrupted") as run (run.id)}<Feedback kind="warning" title={`Agent run ${run.state}`} message={run.failure||"Send a message to explicitly retry. Open Tasks to inspect previously approved work."} />{/each}
      {#if running}<p role="status" class="text-sm text-muted-foreground">The agent is researching your request. You can return later; this conversation is saved.</p>{/if}
      </div>
      {#if active}<form method="POST" action="?/sendMessage" use:enhance={submit} class="shrink-0 space-y-3"><input type="hidden" name="idempotencyKey" value={key(`message-${conversation.id}-${conversation.updateCursor}`)} /><input type="hidden" name="conversationId" value={conversation.id} /><Label for="project-message">Message</Label><Textarea bind:ref={composer} id="project-message" name="content" rows={3} required maxlength={16384} bind:value={content} placeholder="Describe the work, ask a question, or request a revision…" /><div class="flex flex-wrap justify-between gap-3"><p class="text-xs text-muted-foreground">To cancel or retry work, explicitly request the action here and approve its confirmation card.</p><Button type="submit" disabled={pending||running||!content.trim()||!data.agentConfiguration?.effective.projectModel||!data.agentConfiguration?.effective.codingModel}>{pending?"Sending…":"Send message"}</Button></div>{#if !data.agentConfiguration?.effective.projectModel||!data.agentConfiguration?.effective.codingModel}<p role="status" class="text-sm text-muted-foreground">Choose project and coding agent models in Settings or workspace defaults before sending a message.</p>{/if}</form>{/if}
    </Card.Content></Card.Root>
    {#if data.view==="overview"}{#each conversation.proposals.filter(p=>p.action!=="create_task") as proposal (proposal.id)}
      <ProjectProposal {data} {proposal} allowDecision={proposal.action!=="create_task"} />
    {/each}{/if}
  {:else if !data.conversationError}<p class="text-sm text-muted-foreground">Start a conversation to research a project change and prepare tasks for approval.</p>{/if}
</section>
