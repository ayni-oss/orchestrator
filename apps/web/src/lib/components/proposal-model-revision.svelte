<script lang="ts">
  import {untrack} from "svelte";
  import type {PageData} from "../../routes/projects/[projectId]/$types";
  import {enhance} from "$lib/forms.js";
  import {Button} from "$lib/components/ui/button/index.js";
  import {Label} from "$lib/components/ui/label/index.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import Section from "$lib/components/ui/collapsible/section.svelte";
  let {proposal,connections,catalogues,conversationId,idempotencyKey}:{proposal:PageData["conversations"][number]["proposals"][number];connections:PageData["llmConnections"];catalogues:PageData["connectionModels"];conversationId:string;idempotencyKey:string}=$props();
  let connection=$state(untrack(()=>proposal.request?.connectionId??"")),model=$state(untrack(()=>proposal.request?.agent.model??"")),pending=$state(false);
</script>
<Section title="Change coding model">
  <form method="POST" action="?/reviseModel" use:enhance={()=>{pending=true;return async({update})=>{try{await update({reset:false});}finally{pending=false;}};}} class="mt-3 space-y-3">
    <input type="hidden" name="idempotencyKey" value={idempotencyKey} /><input type="hidden" name="conversationId" value={conversationId} /><input type="hidden" name="proposalId" value={proposal.id} />
    <Label for={`override-connection-${proposal.id}`}>Coding connection for revised plan</Label><ThemedSelect id={`override-connection-${proposal.id}`} name="connectionId" value={connection} onValueChange={value=>{connection=value;model="";}} options={connections.map(c=>({value:c.id,label:c.displayName}))} required placeholder="Choose connection" />
    <Label for={`override-model-${proposal.id}`}>Coding model for revised plan</Label><ThemedSelect id={`override-model-${proposal.id}`} name="model" bind:value={model} options={(catalogues[connection]?.models??[]).map(m=>({value:m.id,label:m.id}))} required placeholder="Choose available model" />
    <p class="text-xs text-muted-foreground">The agent will prepare a new plan with this model for your approval.</p><Button type="submit" variant="outline" disabled={pending||!connection||!model}>Revise coding model</Button>
  </form>
</Section>
