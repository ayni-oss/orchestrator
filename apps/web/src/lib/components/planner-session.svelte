<script lang="ts">
  import type {SubmitFunction} from "@sveltejs/kit";
  import Feedback from "$lib/components/feedback.svelte";
  import {untrack} from "svelte";
  import type {PageData} from "../../routes/projects/[projectId]/$types";
  import {enhance} from "$lib/forms.js";
  import {Button} from "$lib/components/ui/button/index.js";
  import {Label} from "$lib/components/ui/label/index.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import {formatDate,words} from "$lib/presentation.js";
  let {data}:{data:PageData}=$props();
  const conversation=$derived(data.conversation!);
  const session=$derived(conversation.plannerSession);
  const cancellableRun=$derived(conversation.runs.find(run=>run.state==="queued"||run.state==="running"));
  const active=$derived(data.project?.state==="active"&&data.workspace?.state==="active");
  let selected=$state(untrack(()=>data.conversation?.plannerIdleTimeoutSeconds===undefined?"inherit":String(data.conversation.plannerIdleTimeoutSeconds)));
  let pending=$state(false);
  let error=$state(""),success=$state("");
  const submit:SubmitFunction=()=>{pending=true;error="";success="";return async({result,update})=>{try{if(result.type==="failure")error=String(result.data?.error?.message??"Planner settings could not be saved. Reload and try again.");if(result.type==="success")success=String(result.data?.success??"Planner settings saved.");await update({reset:false});}finally{pending=false;}};};
  const inheritedLabel=$derived(`Workspace default (${data.workspace?.plannerIdleTimeoutSeconds===86400?"24 hours":"5 minutes"})`);
  const timeoutLabel=$derived(conversation.effectivePlannerIdleTimeoutSeconds===86400?"24 hours":"5 minutes");
</script>
<section class="rounded-lg border p-4 text-sm" aria-label="Planner session">
  <div class="flex flex-wrap items-center justify-between gap-2"><h3 class="font-medium">Planner session</h3><span role="status">{session?(session.state==="idle"?"Warm":words(session.state)):cancellableRun?words(cancellableRun.state):"Not started"}</span></div>
  {#if session}
    <p class="mt-2 text-muted-foreground">Session generation {session.generation}{session.lastActivityAt?` · Last activity ${formatDate(session.lastActivityAt)}`:""}</p>
    {#if session.state==="idle"}<p class="mt-2">Ready to continue in the retained planner workspace.{#if session.idleExpiresAt} Reusable until <time datetime={session.idleExpiresAt}>{formatDate(session.idleExpiresAt)}</time>.{/if}</p>
    {:else if session.state==="starting"}<p class="mt-2">Preparing a planner session for the current turn.</p>
    {:else if session.state==="stopping"}<p class="mt-2">The previous planner session is stopping before a fresh session can start.</p>
    {:else if session.state==="active"}<p class="mt-2">The planner is working on the current turn. Its idle timeout starts after the turn finishes.</p>
    {:else if session.state==="expired"}<p class="mt-2">The idle session expired. Your next message starts a fresh planner session.</p>
    {:else if session.state==="lost"||session.state==="failed"}<p class="mt-2">The planner session ended unexpectedly. Send a new message to start a fresh session.</p>{/if}
  {:else if cancellableRun}<p class="mt-2">The planner turn is {words(cancellableRun.state).toLowerCase()}. You can cancel it while waiting for a session.</p>
  {:else}<p class="mt-2">Your first message starts a planner session.</p>{/if}
  <p class="mt-2 text-muted-foreground">Idle timeout: {timeoutLabel} · {conversation.plannerIdleTimeoutSeconds===undefined?"inherited from workspace":"conversation override"}. Saved history and plan versions remain available after a session ends.</p>
  {#if active}
    <details class="mt-3"><summary class="cursor-pointer">Planner session settings</summary>
      <form method="POST" action="?/plannerSettings" use:enhance={submit} class="mt-3 space-y-3">
        <input type="hidden" name="idempotencyKey" value={`${data.idempotencyKey}-planner-${conversation.updateCursor}`} /><input type="hidden" name="conversationId" value={conversation.id} /><input type="hidden" name="expectedUpdateCursor" value={conversation.updateCursor} />
        <Label for={`planner-timeout-${conversation.id}`}>Conversation planner idle timeout</Label><ThemedSelect id={`planner-timeout-${conversation.id}`} name="plannerIdleTimeoutSeconds" bind:value={selected} options={[{value:"inherit",label:inheritedLabel},{value:"300",label:"5 minutes"},{value:"86400",label:"24 hours"}]} required />
        <Button type="submit" variant="outline" disabled={pending}>{pending?"Saving…":"Save planner settings"}</Button>
      </form>
    </details>
  {/if}
    {#if cancellableRun}<form method="POST" action="?/cancelPlannerTurn" use:enhance={submit} class="mt-3"><input type="hidden" name="idempotencyKey" value={`${data.idempotencyKey}-cancel-${cancellableRun.id}`} /><input type="hidden" name="conversationId" value={conversation.id} /><input type="hidden" name="runId" value={cancellableRun.id} /><Button type="submit" variant="outline" disabled={pending}>Cancel planner turn</Button><p class="mt-2 text-muted-foreground">Stops the current research turn. Approved coding tasks keep their own controls.</p></form>{/if}
{#if error}<Feedback kind="error" title="Planner settings not saved" message={error} />{:else if success}<p role="status" class="mt-2">{success}</p>{/if}
</section>
