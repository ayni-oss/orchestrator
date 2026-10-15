<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { invalidate } from "$app/navigation";
  import type { PageData } from "../../routes/projects/[projectId]/$types";
  import type { SubmitFunction } from "@sveltejs/kit";
  import { enhance } from "$lib/forms.js";
  import ProposalModelRevision from "$lib/components/proposal-model-revision.svelte";
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
  function actionTitle(action:string){return action==="create_task"?"Task plan":action==="cancel_task"?"Confirm cancellation":action==="retry_task"?"Confirm task retry":action==="retry_publication"?"Confirm publication retry":words(action);}
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
    {#each conversation.proposals as proposal (proposal.id)}
      <InformationCard title={actionTitle(proposal.action)} status={proposal.state}>
        {#if proposal.request && proposal.action==="create_task"}
          {@const request=proposal.request}
          <InformationTable rows={[
            {label:"Repository",value:name(request.repositoryConnectionId)},
            {label:"Source commit",value:request.repositoryRevision,monospace:true},
            {label:"Runtime",value:request.runtimeProfileId},
            ...(proposal.runtime ? [{label:"Runtime revision",value:proposal.runtime.profileRevisionId},{label:"Runtime image",value:`${proposal.runtime.image}@${proposal.runtime.digest}`}]:[]),
            {label:"Coding connection",value:data.llmConnections.find(c=>c.id===request.connectionId)?.displayName??request.connectionId??"Deployment default"},
            {label:"Provider / model",value:`${request.agent.provider} / ${request.agent.model}`},
            {label:"Model turns",value:String(request.codingTurns??24)},
            {label:"Deadline",value:`${request.deadlineSeconds??3600} seconds`},
            {label:"Protected paths",value:request.changeConstraints.protectedPaths.join(", ")||"Default protections"},
            {label:"Test edits",value:request.changeConstraints.allowTestEdits?"Allowed":"Protected"},
          ]} />
          <div class="space-y-2"><h4 class="text-sm font-semibold">Implementation request</h4><p class="whitespace-pre-wrap break-words text-sm">{request.request}</p><h4 class="text-sm font-semibold">Acceptance criteria</h4><ul class="list-disc space-y-1 pl-5 text-sm">{#each request.acceptanceCriteria as criterion}<li>{criterion}</li>{/each}</ul></div>
        {:else}<p class="text-sm">{proposal.action==="cancel_task"?"Cancel the running task and retain its history.":proposal.action==="retry_publication"?"Retry publishing the retained result; coding will not run again.":"Start a new coding attempt using this task’s frozen inputs."} <a class="underline" href={`/tasks/${encodeURIComponent(proposal.targetTaskId)}`}>Open target task</a></p>{/if}
        {#if proposal.failure}<Feedback kind="warning" title="Action needs attention" message={proposal.failure} />{/if}
        {#if proposal.state==="pending" && active}
          <form method="POST" action="?/decideProposal" use:enhance={submit} class="flex flex-wrap gap-2"><input type="hidden" name="idempotencyKey" value={key(`decision-${proposal.id}-${conversation.updateCursor}`)} /><input type="hidden" name="conversationId" value={conversation.id} /><input type="hidden" name="proposalId" value={proposal.id} /><Button type="submit" name="decision" value="approve" disabled={pending}>{proposal.action==="create_task"?"Approve and submit task":"Confirm action"}</Button><Button type="submit" name="decision" value="revise" variant="outline" disabled={pending}>Request revision</Button><Button type="submit" name="decision" value="reject" variant="outline" disabled={pending}>Reject</Button></form>
          {#if proposal.action==="create_task" && proposal.request}<ProposalModelRevision {proposal} connections={data.llmConnections} catalogues={data.connectionModels} conversationId={conversation.id} idempotencyKey={key(`model-${proposal.id}-${conversation.updateCursor}`)} />{/if}
          <p class="text-xs text-muted-foreground">Approval submits these exact requirements. For a different coding model, connection, or scope, request a revision and send the changes in a message.</p>
        {/if}
        {#if proposal.taskId}
          {@const task=data.tasks.find(t=>t.id===proposal.taskId)}
          {@const attempt=data.taskProgress[proposal.taskId]?.attempts.at(-1)}
          {@const receipt=attempt?.pullRequest??attempt?.result?.pullRequest}
          <div class="space-y-2 border-t pt-3"><a class="font-medium underline" href={`/tasks/${encodeURIComponent(proposal.taskId)}`}>View task {task?`· ${words(task.state)}`:""}</a>
            {#if attempt}<InformationTable rows={[{label:"Execution",value:attempt.result?.execution??attempt.progress?.phase??words(attempt.state)},{label:"Quality",value:attempt.result?.quality?.outcome??"Pending"},{label:"Publication",value:attempt.result?.publication??attempt.progress?.publication??"Pending"}]} />{#if attempt.requestedInput}<p class="whitespace-pre-wrap text-sm">Input requested: {attempt.requestedInput}</p>{/if}{#if receipt?.url}<Button href={receipt.url} variant="outline" target="_blank" rel="noopener noreferrer">Open draft PR #{receipt.number}</Button>{/if}{/if}
          </div>
        {/if}
      </InformationCard>
    {/each}
  {:else if !data.conversationError}<p class="text-sm text-muted-foreground">Start a conversation to research a project change and prepare tasks for approval.</p>{/if}
</section>
<Dialog.Root bind:open={createOpen}><Dialog.Content><Dialog.Header><Dialog.Title>New conversation</Dialog.Title><Dialog.Description>Select repositories to research. Each approved interaction creates at most one task per repository.</Dialog.Description></Dialog.Header>
  <form method="POST" action="?/createConversation" use:enhance={()=>{pending=true;return async({result,update})=>{try{if(result.type==="redirect")createOpen=false;await update({reset:false});}finally{pending=false;}};}} class="space-y-4"><input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><Label for="conversation-title">Title</Label><Input id="conversation-title" name="title" required maxlength={256} /><Label for="conversation-repositories">Repositories</Label><RepositoryPicker id="conversation-repositories" repositories={data.repositories.filter(repository=>data.workspace?.repositoryConnectionIds.includes(repository.id))} value={members} onchange={value=>members=value} /><Dialog.Footer><Button type="button" variant="outline" onclick={()=>createOpen=false}>Cancel</Button><Button type="submit" disabled={pending||!members.length}>Create conversation</Button></Dialog.Footer></form>
</Dialog.Content></Dialog.Root>
