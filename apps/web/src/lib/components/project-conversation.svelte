<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { invalidate } from "$app/navigation";
  import type { PageData } from "../../routes/projects/[projectId]/$types";
  import type { SubmitFunction } from "@sveltejs/kit";
  import { enhance } from "$lib/forms.js";
  import ProjectProposal from "$lib/components/project-proposal.svelte";
  import ProjectPlanReview from "$lib/components/project-plan-review.svelte";
  import RepositoryPicker from "$lib/components/repository-picker.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { Textarea } from "$lib/components/ui/textarea/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { formatDate, words } from "$lib/presentation.js";
  let {data}:{data:PageData}=$props();
  let createOpen=$state(false),members=$state<string[]>([]),content=$state(""),pending=$state(false),refreshing=false;
  const conversation=$derived(data.conversation);
  const active=$derived(data.project?.state==="active"&&data.workspace?.state==="active");
  const running=$derived(conversation?.runs.some(r=>r.state==="queued"||r.state==="running")??false);
  onMount(()=>{
    const timer=setInterval(async()=>{
      if(refreshing || document.visibilityState!=="visible")return;
      refreshing=true;
      try{await Promise.all([invalidate("app:conversations"),invalidate("app:tasks")]);}finally{refreshing=false;}
    },15000);
    return ()=>clearInterval(timer);
  });
  let live=$state(false);
  const selectedConversationId=$derived(conversation?.id??"");
  const selectedProjectId=$derived(conversation?.projectId??"");
  $effect(()=>{
    if(!selectedConversationId)return;
    const initialCursor=untrack(()=>conversation?.updateCursor??"0");
    const source=new EventSource(`/projects/${encodeURIComponent(selectedProjectId)}/conversations/${encodeURIComponent(selectedConversationId)}/updates?after=${encodeURIComponent(initialCursor)}`);
    source.onopen=()=>{live=true;};
    source.onerror=()=>{live=false;};
    source.addEventListener("changed",()=>{void Promise.all([invalidate("app:conversations"),invalidate("app:tasks")]);});
    return ()=>{source.close();live=false;};
  });
  const submit:SubmitFunction=()=>{pending=true;return async({result,update})=>{try{if(result.type==="success")content="";await update({reset:false});}finally{pending=false;}};};
  function key(suffix:string){return `${data.idempotencyKey}-${suffix}`;}
  function name(id:string){return data.repositories.find(r=>r.id===id)?.displayName??id;}
</script>
<section class="space-y-4" aria-label="Project conversations">
  <div class="flex flex-wrap items-center justify-between gap-3"><h2 class="text-lg font-semibold">Conversations</h2><span class="text-xs text-muted-foreground">{live?"Live updates":"Reconnecting · periodic refresh"}</span>{#if active}<Button variant="outline" onclick={()=>{members=data.project!.repositoryConnectionIds.filter(id=>data.workspace?.repositoryConnectionIds.includes(id));createOpen=true;}}>New conversation</Button>{/if}</div>
  {#if data.conversationError}<Feedback kind="warning" title="Conversations unavailable" message={data.conversationError.message} />{/if}
  {#if data.conversations.length}<nav class="flex flex-wrap gap-2" aria-label="Conversation history">{#each data.conversations as c (c.id)}<Button variant={c.id===conversation?.id?"secondary":"outline"} href={`?conversation=${encodeURIComponent(c.id)}`} aria-current={c.id===conversation?.id?"page":undefined}>{c.title}</Button>{/each}</nav>{/if}
  {#if conversation}
    <InformationCard title={conversation.title} status={running?"running":""}>
      <p class="text-sm text-muted-foreground">Researching {conversation.repositoryConnectionIds.map(name).join(", ")}. Tasks are submitted only after your approval.</p>
      <div class="space-y-4" aria-label="Conversation messages" aria-live="polite" aria-relevant="additions text">
        {#each conversation.messages as message (message.id)}
          <article class={['rounded-lg border p-4',message.role==='user'&&'bg-muted/40']}><div class="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground"><span>{message.role==="user"?"You":message.role==="assistant"?"Project agent":words(message.role)}</span><time datetime={message.createdAt}>{formatDate(message.createdAt)}</time></div><p class="whitespace-pre-wrap break-words text-sm">{message.content}</p></article>
        {:else}<p class="text-sm text-muted-foreground">Describe the change you want. The project agent will research these repositories and clarify the requirements.</p>{/each}
      </div>
      {#if conversation.runs.length}<details class="text-sm"><summary class="cursor-pointer text-muted-foreground">Agent activity · {conversation.runs.length} runs</summary><div class="mt-3 space-y-2">{#each conversation.runs as run (run.id)}<InformationTable rows={[{label:"Run",value:words(run.state)},{label:"Model",value:run.model},{label:"Model calls",value:String(run.modelCalls)},{label:"Total tokens",value:run.usage?.totalTokens??"Unavailable"}]} />{/each}</div></details>{/if}
      {#each conversation.runs.filter(r=>r.state==="failed"||r.state==="interrupted") as run (run.id)}<Feedback kind="warning" title={`Agent run ${words(run.state)}`} message={run.failure||"Send a message to explicitly retry. Previously approved tasks remain linked below."} />{/each}
      {#if running}<p role="status" class="text-sm text-muted-foreground">The agent is researching your request. You can return later; this conversation is saved.</p>{/if}
      {#if active}<form method="POST" action="?/sendMessage" use:enhance={submit} class="space-y-3"><input type="hidden" name="idempotencyKey" value={key(`message-${conversation.id}-${conversation.updateCursor}`)} /><input type="hidden" name="conversationId" value={conversation.id} /><Label for="project-message">Message</Label><Textarea id="project-message" name="content" rows={3} required maxlength={16384} bind:value={content} placeholder="Describe the work, ask a question, or request a revision…" /><div class="flex flex-wrap justify-between gap-3"><p class="text-xs text-muted-foreground">To cancel or retry work, explicitly request the action here and approve its confirmation card.</p><Button type="submit" disabled={pending||running||!content.trim()||!data.project?.agentDefaults}>Send message</Button></div>{#if !data.project?.agentDefaults}<p role="status" class="text-sm text-muted-foreground">Choose project and coding agent models below before sending a message.</p>{/if}</form>{/if}
    </InformationCard>
    {#if conversation.plan}{#key conversation.plan.id}<ProjectPlanReview {data} />{/key}{/if}
    {#each conversation.proposals.filter(p=>p.action!=="create_task"||!conversation.plan) as proposal (proposal.id)}
      <ProjectProposal {data} {proposal} allowDecision={proposal.action!=="create_task"} />
    {/each}
  {:else if !data.conversationError}<p class="text-sm text-muted-foreground">Start a conversation to research a project change and prepare tasks for approval.</p>{/if}
</section>
<Dialog.Root bind:open={createOpen}><Dialog.Content><Dialog.Header><Dialog.Title>New conversation</Dialog.Title><Dialog.Description>Select repositories to research. Each approved interaction creates at most one task per repository.</Dialog.Description></Dialog.Header>
  <form method="POST" action="?/createConversation" use:enhance={()=>{pending=true;return async({result,update})=>{try{if(result.type==="redirect")createOpen=false;await update({reset:false});}finally{pending=false;}};}} class="space-y-4"><input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><Label for="conversation-title">Title</Label><Input id="conversation-title" name="title" required maxlength={256} /><Label for="conversation-repositories">Repositories</Label><RepositoryPicker id="conversation-repositories" repositories={data.repositories.filter(repository=>data.workspace?.repositoryConnectionIds.includes(repository.id))} value={members} onchange={value=>members=value} /><Dialog.Footer><Button type="button" variant="outline" onclick={()=>createOpen=false}>Cancel</Button><Button type="submit" disabled={pending||!members.length}>Create conversation</Button></Dialog.Footer></form>
</Dialog.Content></Dialog.Root>
