<script lang="ts">
  import type {SubmitFunction} from "@sveltejs/kit";
  import Feedback from "$lib/components/feedback.svelte";
  import {untrack} from "svelte";
  import type {ProjectViewData} from "$lib/project-view.js";
  import {enhance} from "$lib/forms.js";
  import {Button} from "$lib/components/ui/button/index.js";
  import {Label} from "$lib/components/ui/label/index.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  let {data}:{data:ProjectViewData}=$props();
  const conversation=$derived(data.conversation!);
  const cancellableRun=$derived(conversation.runs.find(run=>run.state==="queued"||run.state==="running"));
  const active=$derived(data.project?.state==="active"&&data.workspace?.state==="active");
  let selected=$state(untrack(()=>data.conversation?.plannerIdleTimeoutSeconds===undefined?"inherit":String(data.conversation.plannerIdleTimeoutSeconds)));
  let pending=$state(false);
  let error=$state(""),success=$state("");
  const submit:SubmitFunction=()=>{pending=true;error="";success="";return async({result,update})=>{try{if(result.type==="failure")error=String(result.data?.error?.message??"Planner settings could not be saved. Reload and try again.");if(result.type==="success")success=String(result.data?.success??"Planner settings saved.");await update({reset:false});}finally{pending=false;}};};
  const inheritedLabel=$derived(`Workspace default (${data.workspace?.plannerIdleTimeoutSeconds===86400?"24 hours":"5 minutes"})`);
</script>
<InformationCard title="Conversation settings">
  <p class="text-sm text-muted-foreground">Planner timeout for {conversation.title}. Saved messages remain available when an idle session ends.</p>
  {#if active}
      <form method="POST" action="?/plannerSettings" use:enhance={submit} class="mt-3 space-y-3">
        <input type="hidden" name="idempotencyKey" value={`${data.idempotencyKey}-planner-${conversation.updateCursor}`} /><input type="hidden" name="conversationId" value={conversation.id} /><input type="hidden" name="expectedUpdateCursor" value={conversation.updateCursor} />
        <Label for={`planner-timeout-${conversation.id}`}>Conversation planner idle timeout</Label><ThemedSelect id={`planner-timeout-${conversation.id}`} name="plannerIdleTimeoutSeconds" bind:value={selected} options={[{value:"inherit",label:inheritedLabel},{value:"300",label:"5 minutes"},{value:"86400",label:"24 hours"}]} required />
        <Button type="submit" variant="outline" disabled={pending}>{pending?"Saving…":"Save planner settings"}</Button>
      </form>
  {/if}
    {#if cancellableRun}<form method="POST" action="?/cancelPlannerTurn" use:enhance={submit} class="mt-3"><input type="hidden" name="idempotencyKey" value={`${data.idempotencyKey}-cancel-${cancellableRun.id}`} /><input type="hidden" name="conversationId" value={conversation.id} /><input type="hidden" name="runId" value={cancellableRun.id} /><Button type="submit" variant="outline" disabled={pending}>Cancel planner turn</Button><p class="mt-2 text-muted-foreground">Stops the current research turn. Approved coding tasks keep their own controls.</p></form>{/if}
{#if error}<Feedback kind="error" title="Planner settings not saved" message={error} />{:else if success}<p role="status" class="mt-2">{success}</p>{/if}
</InformationCard>
