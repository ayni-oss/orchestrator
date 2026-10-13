<script lang="ts">
  import { onMount } from "svelte";
  import type { SubmitFunction } from "@sveltejs/kit";
  import type { ConnectionView } from "$lib/connection.js";
  import { enhance } from "$lib/forms.js";
  import { refreshScope, subscribeRefresh } from "$lib/refresh.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  let {connections,idempotencyKey,available=true}:{connections:ConnectionView[];idempotencyKey:string;available?:boolean}=$props();
  let createOpen=$state(false), editOpen=$state(false), selectedId=$state(""), apiKey=$state(""), pending=$state(false);
  const selected=$derived(connections.find(c=>c.id===selectedId));
  onMount(()=>subscribeRefresh("app:connections",{intervalMs:30_000}));
  function closeCreate(open:boolean){createOpen=open;apiKey="";}
  function closeEdit(open:boolean){editOpen=open;apiKey="";}
  function submit(operation:string):SubmitFunction{return ()=>{pending=true;return async({result,update})=>{apiKey="";try{await update({invalidateAll:false});if(result.type==="success"&&operation==="create")closeCreate(false);if(result.type==="success"&&operation==="delete")closeEdit(false);await refreshScope("app:connections",true);}finally{pending=false;}};};}
</script>
<div class="space-y-4">
  <div class="flex items-center justify-between gap-3"><h2 class="text-lg font-semibold">Connections</h2><Button disabled={!available} onclick={()=>{apiKey="";createOpen=true;}}>Add connection</Button></div>
  {#if !available}<p role="status">Connection service unavailable. Retry when the control plane is ready.</p>{/if}
  {#each connections as connection (connection.id)}
    <InformationCard title={connection.displayName} status={connection.enabled?connection.status:"disabled"} actions={[{label:connection.managementSource==="admin"?"Details":"Edit",onSelect:()=>{selectedId=connection.id;apiKey="";editOpen=true;}}]}>
      <InformationTable rows={[{label:"Authentication",value:connection.testAuthentication||"Not tested"},...(connection.managementSource==="admin"?[{label:"Managed by",value:"Your administrator"}]:[])]} />
      {#if connection.failureReason}<Feedback kind="warning" title="Connection needs attention" message={connection.failureReason} />{/if}
    </InformationCard>
  {:else}<p class="text-sm text-muted-foreground">Add an OpenAI connection, then choose models in your project settings.</p>{/each}
</div>
<Dialog.Root open={createOpen} onOpenChange={closeCreate}><Dialog.Content><Dialog.Header><Dialog.Title>Add OpenAI connection</Dialog.Title><Dialog.Description>Your API key is stored in Kubernetes and never returned by this page.</Dialog.Description></Dialog.Header>
  <form method="POST" action="?/create" use:enhance={submit("create")} class="space-y-4">
    <input type="hidden" name="idempotencyKey" value={idempotencyKey} />
    <div class="space-y-2"><Label for="llm-name">Connection name</Label><Input id="llm-name" name="displayName" maxlength={128} required /></div>
    <div class="space-y-2"><Label for="llm-key">API key</Label><Input id="llm-key" name="apiKey" type="password" autocomplete="new-password" required bind:value={apiKey} /></div>
    <Dialog.Footer><Button type="button" variant="outline" onclick={()=>closeCreate(false)}>Cancel</Button><Button type="submit" disabled={pending}>Save and test</Button></Dialog.Footer>
  </form>
</Dialog.Content></Dialog.Root>
<Dialog.Root open={editOpen} onOpenChange={closeEdit}><Dialog.Content><Dialog.Header><Dialog.Title>{selected?.managementSource==="admin"?"Connection details":"Edit OpenAI connection"}</Dialog.Title><Dialog.Description>{selected?.displayName??"This connection is no longer available."}</Dialog.Description></Dialog.Header>
  {#if selected}
    {#if selected.managementSource==="admin"}<p class="text-sm">Managed by your administrator. Update credentials in the deployment’s secret store.</p>
    {:else}<form method="POST" action="?/replace" use:enhance={submit("replace")} class="space-y-4">
      <input type="hidden" name="id" value={selected.id} /><input type="hidden" name="idempotencyKey" value={`${idempotencyKey}-replace-${selected.id}`} /><input type="hidden" name="resourceVersion" value={selected.resourceVersion} />
      <Label for="llm-replacement">New API key</Label><Input id="llm-replacement" name="apiKey" type="password" autocomplete="new-password" required bind:value={apiKey} />
      <p class="text-sm text-muted-foreground">The current credentials remain active if the replacement fails validation. Existing work retains its admitted credential revision.</p><Button type="submit" disabled={pending}>Replace credentials</Button>
    </form>{/if}
    <div class="flex flex-wrap gap-2 border-t pt-4">
      {#each (selected.managementSource==="admin"?["test"]:["test","toggle","delete"]) as operation (operation)}
        <form method="POST" action={`?/${operation}`} use:enhance={submit(operation)}><input type="hidden" name="id" value={selected.id} /><input type="hidden" name="idempotencyKey" value={`${idempotencyKey}-${operation}-${selected.id}`} /><input type="hidden" name="resourceVersion" value={selected.resourceVersion} /><input type="hidden" name="enabled" value={String(!selected.enabled)} /><Button type="submit" variant={operation==="delete"?"destructive":"outline"} disabled={pending}>{operation==="test"?"Test connection":operation==="toggle"?selected.enabled?"Disable":"Enable":"Delete"}</Button></form>
      {/each}
    </div>
  {/if}
</Dialog.Content></Dialog.Root>
