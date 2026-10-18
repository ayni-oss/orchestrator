<script lang="ts">
  import type {ProjectViewData} from "$lib/project-view.js";
  import type {ConversationHistoryDto,ConversationRecordDto} from "$lib/server/conversation-history.js";
  import {Button} from "$lib/components/ui/button/index.js";
  import Feedback from "$lib/components/feedback.svelte";
  import {formatDate,words} from "$lib/presentation.js";
  let {conversation}:{conversation:NonNullable<ProjectViewData["conversation"]>}=$props();
  let history=$state<ConversationHistoryDto>(),older=$state<ConversationRecordDto[]>([]),pending=$state(false),error=$state("");
  const messages=$derived(older.filter(record=>!conversation.messages.some(message=>message.id===record.sourceId)));
  async function loadEarlier(){
    pending=true;error="";
    const params=new URLSearchParams({limit:"50"});
    if(history){params.set("before",history.nextBeforeSequence);params.set("snapshot",history.snapshotSequence);}
    try {
      const response=await fetch(`/_old/projects/${encodeURIComponent(conversation.projectId)}/conversations/${encodeURIComponent(conversation.id)}/history?${params}`),result=await response.json();
      if(!response.ok)throw new Error(result.error?.message??"Earlier messages could not be loaded. Try again.");
      history=result;
      older=[...older,...result.records.filter((record:ConversationRecordDto)=>record.kind==="message"&&(record.role==="user"||record.role==="assistant"))].sort((a,b)=>BigInt(a.sequence)<BigInt(b.sequence)?-1:1);
    }catch(cause){error=cause instanceof Error?cause.message:"Earlier messages could not be loaded. Try again.";}
    finally{pending=false;}
  }
</script>
{#if !history||history.hasMore}<Button type="button" variant="ghost" size="sm" onclick={loadEarlier} disabled={pending}>{pending?"Loading…":"Load earlier messages"}</Button>{/if}
{#if error}<Feedback kind="warning" title="Messages unavailable" message={error} />{/if}
<div class="space-y-4" aria-label="Conversation messages" aria-live="polite" aria-relevant="additions text">
  {#each messages as message (message.id)}
    <article class={['rounded-lg border p-4',message.role==='user'&&'bg-muted/40']}><p class="mb-2 text-xs text-muted-foreground">{message.role==="user"?"You":"Project agent"}</p><p class="whitespace-pre-wrap break-words text-sm">{message.content}</p></article>
  {/each}
  {#each conversation.messages as message (message.id)}
    <article class={['rounded-lg border p-4',message.role==='user'&&'bg-muted/40']}><div class="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground"><span>{message.role==="user"?"You":message.role==="assistant"?"Project agent":words(message.role)}</span><time datetime={message.createdAt}>{formatDate(message.createdAt)}</time></div><p class="whitespace-pre-wrap break-words text-sm">{message.content}</p></article>
  {:else}<p class="text-sm text-muted-foreground">Describe the change you want. The project agent will research the selected repositories and clarify the requirements.</p>{/each}
</div>
