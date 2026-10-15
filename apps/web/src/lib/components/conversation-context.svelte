<script lang="ts">
  import type {ConversationContextDto,ConversationHistoryDto,ConversationRecordDto} from "$lib/server/conversation-history.js";
  import {Button} from "$lib/components/ui/button/index.js";
  import {Input} from "$lib/components/ui/input/index.js";
  import {Label} from "$lib/components/ui/label/index.js";
  import Feedback from "$lib/components/feedback.svelte";
  import {words} from "$lib/presentation.js";
  let {projectId,conversationId,updateCursor}:{projectId:string;conversationId:string;updateCursor:string}=$props();
  const base=$derived(`/projects/${encodeURIComponent(projectId)}/conversations/${encodeURIComponent(conversationId)}`);
  let context=$state<ConversationContextDto>(),contextError=$state("");
  let query=$state(""),searchedQuery=$state(""),records=$state<ConversationRecordDto[]>([]),history=$state<ConversationHistoryDto>(),historyError=$state(""),pending=$state(false),opened=$state(false);
  $effect(()=>{
    void updateCursor;
    const controller=new AbortController();
    void fetch(`${base}/context`,{signal:controller.signal}).then(async response=>{
      const result=await response.json();
      if(!response.ok)throw new Error(result.error?.message??"Saved context is unavailable. Reload to try again.");
      context=result;contextError="";
    }).catch(error=>{if(!controller.signal.aborted)contextError=error.message;});
    return ()=>controller.abort();
  });
  async function retrieve(older=false){
    pending=true;historyError="";
    const params=new URLSearchParams({q:older?searchedQuery:query,limit:"25"});
    if(older&&history){params.set("before",history.nextBeforeSequence);params.set("snapshot",history.snapshotSequence);}
    try{
      const response=await fetch(`${base}/history?${params}`),result=await response.json();
      if(!response.ok)throw new Error(result.error?.message??"Saved history is unavailable. Try again.");
      history=result;records=older?[...records,...result.records]:result.records;if(!older)searchedQuery=query;opened=true;
    }catch(error){historyError=error instanceof Error?error.message:"Saved history is unavailable. Try again.";}
    finally{pending=false;}
  }
</script>
<details class="rounded-lg border p-4" aria-label="Saved planning context">
  <summary class="cursor-pointer text-sm font-medium">Saved planning context and history</summary>
  <div class="mt-4 space-y-4 text-sm">
    {#if contextError}<Feedback kind="warning" title="Planning context unavailable" message={contextError} />
    {:else if context}
      <p>{context.archiveRecordCount} original records saved. The conversation shows the latest {context.recentMessageLimit} messages.</p>
      {#if context.summary}
        <p class="font-medium">Working summary · version {context.summary.version}</p>
        <p class="text-muted-foreground">This summary is an excerpt index of saved records. It is not exhaustive; search the original history for details.</p>
        <p class="whitespace-pre-wrap break-words">{context.summary.content}</p>
        <p class="text-muted-foreground">Summary covers records through {context.summary.sourceWatermark}; archive watermark {context.sourceWatermark}. {context.summary.sourceWatermark===context.sourceWatermark?"Summary watermark is current.":"Newer records are available in saved history."}</p>
        <details><summary class="cursor-pointer">Summary source references ({context.summary.sourceRecordIds.length})</summary><ul class="mt-2 list-inside list-disc break-words">{#each context.summary.sourceRecordIds as id (id)}<li>{id}</li>{/each}</ul></details>
      {:else}<p>No working summary yet. Original records remain available in saved history.</p>{/if}
      <p class="text-muted-foreground">The archive preserves up to {context.capacityRecordLimit} original records. The agent receives a bounded selection of original records and the working summary; saved history remains browsable and searchable. Summary references identify its sources, not a guarantee that every detail fits in model context.</p>
    {:else}<p role="status">Loading saved context…</p>{/if}
    <form class="space-y-2" onsubmit={event=>{event.preventDefault();void retrieve();}}>
      <Label for={`history-search-${conversationId}`}>Search saved history</Label>
      <Input id={`history-search-${conversationId}`} bind:value={query} maxlength={256} placeholder="Find an earlier requirement…" />
      <div class="flex flex-wrap gap-2"><Button type="submit" variant="outline" disabled={pending}>Search history</Button><Button type="button" variant="outline" disabled={pending} onclick={()=>{query="";void retrieve();}}>Browse saved history</Button></div>
    </form>
    {#if historyError}<Feedback kind="warning" title="History unavailable" message={historyError} />{/if}
    {#if opened&&history}
      <p class="text-muted-foreground">{searchedQuery?`Matches for “${searchedQuery}”`:"Original records, newest first"} · snapshot through {history.snapshotSequence} · {history.archiveRecordCount} archived records.</p>
      <div class="space-y-2" aria-label="Saved history records">{#each records as record (record.id)}<article class="rounded border p-3"><p class="mb-2 text-xs text-muted-foreground">Record {record.sequence} · {words(record.kind)}{record.role?` · ${words(record.role)}`:""}{record.repositoryId?` · ${record.repositoryId}`:""}</p><p class="whitespace-pre-wrap break-words">{record.content}</p><p class="mt-2 break-all text-xs text-muted-foreground">Source: {record.id}</p></article>{:else}<p>No saved records match this search.</p>{/each}</div>
      {#if history.hasMore}<Button variant="outline" disabled={pending} onclick={()=>retrieve(true)}>Load older records</Button>{:else}<p role="status" class="text-muted-foreground">{searchedQuery?"All matching records loaded.":"All saved records loaded."}</p>{/if}
    {/if}
  </div>
</details>
