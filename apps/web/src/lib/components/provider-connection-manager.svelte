<script lang="ts">
 import { onMount } from 'svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import * as Dialog from '$lib/components/ui/dialog/index.js';
 import InformationCard from '$lib/components/information-card.svelte';
 import InformationTable from '$lib/components/information-table.svelte';
 import Feedback from '$lib/components/feedback.svelte';
 import { rpc, RpcError, reasonText, type Connection, type Operation } from '$lib/api';
 let { capability }: { capability: 'llm' | 'container_registry' } = $props();
 const registry=$derived(capability==='container_registry');
 const provider=$derived(registry?'GitHub Container Registry':'OpenAI');
 const storageKey=$derived(`ayni.connection.${capability}.pending`);
 let connections=$state.raw<Connection[]>([]); let history=$state.raw<Operation[]>([]);
 let selected=$state<Connection|null>(null);
 let name=$state(''); let destination=$state(''); let pushUsername=$state(''); let pullUsername=$state('');
 let pushToken=$state(''); let pullToken=$state(''); let apiKey=$state('');
 let open=$state(false); let busy=$state(true); let error=$state(''); let notice=$state('');
 let pendingKey=$state(''); let next=$state(''); let historyNext=$state(''); let generation=0;
 function clearCredentials(){pushToken='';pullToken='';apiKey='';}
 function persist(key:string){pendingKey=key;if(key)sessionStorage.setItem(storageKey,key);else sessionStorage.removeItem(storageKey);}
 async function perform(action:()=>Promise<void>){busy=true;error='';try{await action();}catch(e){error=e instanceof Error?e.message:'Service unavailable. Refresh and retry.';}finally{busy=false;}}
 async function load(append=false){const result=await rpc('ListConnections',{afterId:append?next:'',limit:50});const values=(result.connections||[]).filter((c:Connection)=>c.capability===capability);connections=append?[...connections,...values]:values;next=result.nextAfterId||'';}
 async function inspect(c:Connection,append=false){const current=++generation;if(!append){selected=c;name=c.displayName;destination=c.registryDestination||'';pushUsername=c.pushUsername||'';pullUsername=c.pullUsername||'';clearCredentials();history=[];}const result=await rpc('ListConnectionOperations',{connectionId:c.id,afterId:append?historyNext:'',limit:50});if(current!==generation)return;history=append?[...history,...(result.operations||[])]:result.operations||[];historyNext=result.nextAfterId||'';}
 async function refresh(){await load();if(selected){const c=connections.find(c=>c.id===selected?.id);if(c)await inspect(c);}}
 async function accept(op:Operation){await load();await inspect(op.connection);notice=op.kind==='enable'?(op.connection.enabled?'Connection enabled.':'Connection disabled.'):op.state==='completed'?'Credentials validated and activated.':op.state==='failed'?'Candidate validation failed. The previous active credential is preserved.':op.state==='superseded'?'This operation was superseded. Refresh before making changes.':'Credentials saved. Validate the candidate before use.';if(op.safeReason)notice+=` ${reasonText[op.safeReason]||op.safeReason}`;}
 function create(){++generation;selected=null;history=[];name='';destination='';pushUsername='';pullUsername='';clearCredentials();error='';notice='';open=true;}
 async function save(event:SubmitEvent){event.preventDefault();if(pendingKey)return;const key=crypto.randomUUID();const request={idempotencyKey:key,connectionId:selected?.id||'',expectedRevision:selected?.revision||'0',displayName:name,provider:registry?'github':'openai',capability,registryDestination:registry?destination:'',pushUsername:registry?pushUsername:'',pullUsername:registry?pullUsername:'',pushToken,pullToken,apiKey};clearCredentials();persist(key);await perform(async()=>{try{const result=await rpc('SaveProviderConnection',request);persist('');open=false;await accept(result.operation);}catch(e){if(e instanceof RpcError && e.status===400)persist('');throw e;}finally{request.pushToken='';request.pullToken='';request.apiKey='';}});}
 async function recover(){await perform(async()=>{const result=await rpc('GetConnectionOperation',{idempotencyKey:pendingKey});persist('');open=false;await accept(result.operation);});}
 async function toggle(){if(!selected)return;const key=crypto.randomUUID();persist(key);const result=await rpc('SetConnectionEnabled',{idempotencyKey:key,connectionId:selected.id,expectedRevision:selected.revision,enabled:!selected.enabled});persist('');await accept(result.operation);}
 onMount(()=>{pendingKey=sessionStorage.getItem(storageKey)||'';void perform(refresh);});
</script>
<div class="space-y-4">
 <div class="flex items-center justify-between gap-3"><h2 class="text-lg font-semibold">Connections</h2><Button disabled={busy||!!pendingKey} onclick={create}>Add {provider} connection</Button></div>
 {#if error}<Feedback kind="error" title="Connection needs attention" message={error}/>{/if}
 {#if notice}<Feedback title="Connection update" message={notice}/>{/if}
 {#if pendingKey}<InformationCard title="Recover a submitted command"><p>Check the retained receipt before submitting credentials again.</p><code>{pendingKey}</code><div class="flex gap-3"><Button disabled={busy} onclick={recover}>Recover operation</Button><Button variant="outline" disabled={busy} onclick={()=>persist('')}>Abandon recovery</Button></div></InformationCard>{/if}
 {#if !connections.length}<p class="text-sm text-muted-foreground">Add a named {provider} connection to get started.</p>{/if}
 {#each connections as c (c.id)}<InformationCard title={c.displayName} status={c.status}>
  <InformationTable rows={[{label:registry?'Destination':'Endpoint',value:registry?(c.registryDestination||''):'api.openai.com'},{label:'Credential',value:c.activeCredentialRevision?'Configured':'No active credential'},{label:'Revision',value:c.revision}]}/>
  {#if registry}<InformationTable rows={[{label:'Push account',value:c.pushUsername||''},{label:'Pull account',value:c.pullUsername||''}]}/>{/if}
  {#if c.safeReason}<Feedback kind="warning" title="Latest operation needs attention" message={reasonText[c.safeReason]||c.safeReason}/>{/if}
  <div class="flex flex-wrap gap-3"><Button variant="outline" disabled={busy} onclick={()=>perform(async()=>{await inspect(c);open=true;})}>Edit {c.displayName}</Button><Button variant="outline" disabled={busy} onclick={()=>perform(()=>inspect(c))}>Inspect {c.displayName}</Button></div>
 </InformationCard>{/each}
 {#if next}<Button variant="outline" disabled={busy} onclick={()=>perform(()=>load(true))}>Load more connections</Button>{/if}
 <Button variant="outline" disabled={busy} onclick={()=>perform(refresh)}>Refresh connections</Button>
</div>
<Dialog.Root bind:open onOpenChange={value=>{if(!value)clearCredentials();}}><Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl"><Dialog.Header><Dialog.Title>{selected?'Connection settings':`Add ${provider} connection`}</Dialog.Title><Dialog.Description>{registry?'Use GitHub personal access tokens (classic). Enter push and pull credentials separately.':'Use an OpenAI API key. Validation discovers models without running a paid conversation.'}</Dialog.Description></Dialog.Header>
 {#if error}<Feedback kind="error" title="Connection could not be saved" message={error}/>{/if}
 {#if pendingKey}<InformationCard title="Recover a submitted command"><p>The save result is uncertain. Recover it before submitting credentials again.</p><div class="flex gap-3"><Button disabled={busy} onclick={recover}>Recover operation</Button><Button variant="outline" disabled={busy} onclick={()=>persist('')}>Abandon recovery</Button></div></InformationCard>{/if}
 {#if busy}<p role="status">Saving connection…</p>{/if}
 <form onsubmit={save} class="space-y-5" autocomplete="off">
  <div class="space-y-2"><Label for="provider-name">Connection name</Label><Input id="provider-name" bind:value={name} required maxlength={120} disabled={busy||!!pendingKey}/></div>
  {#if registry}
   <div class="space-y-2"><Label for="registry-destination">Destination</Label><Input id="registry-destination" bind:value={destination} placeholder="ghcr.io/owner/package" pattern="ghcr\.io/[a-z0-9][a-z0-9._\-]*(/[a-z0-9][a-z0-9._\-]*)+" title="Enter a full image destination, such as ghcr.io/owner/package." required maxlength={208} disabled={busy||!!pendingKey||!!selected}/><p class="text-xs text-muted-foreground">Include the owner and package name, for example ghcr.io/ayni-oss/runtime.</p></div>
   <div class="space-y-2"><Label for="push-username">Push username</Label><Input id="push-username" bind:value={pushUsername} required maxlength={100} disabled={busy||!!pendingKey||!!selected}/></div>
   <div class="space-y-2"><Label for="push-token">{selected?'Replacement push PAT':'Push PAT'}</Label><Input id="push-token" type="password" bind:value={pushToken} required maxlength={4096} autocomplete="new-password" disabled={busy||!!pendingKey}/><p class="text-xs text-muted-foreground">Requires write:packages access to this destination.</p></div>
   <div class="space-y-2"><Label for="pull-username">Pull username</Label><Input id="pull-username" bind:value={pullUsername} required maxlength={100} disabled={busy||!!pendingKey||!!selected}/></div>
   <div class="space-y-2"><Label for="pull-token">{selected?'Replacement pull PAT':'Pull PAT'}</Label><Input id="pull-token" type="password" bind:value={pullToken} required maxlength={4096} autocomplete="new-password" disabled={busy||!!pendingKey}/><p class="text-xs text-muted-foreground">Requires read:packages access. This credential is validated independently.</p></div>
  {:else}<div class="space-y-2"><Label for="api-key">{selected?'Replacement API key':'API key'}</Label><Input id="api-key" type="password" bind:value={apiKey} required maxlength={4096} autocomplete="new-password" disabled={busy||!!pendingKey}/></div>{/if}
  <Dialog.Footer><Button type="button" variant="outline" onclick={()=>open=false}>Cancel</Button><Button type="submit" disabled={busy||!!pendingKey}>{selected?'Save candidate credentials':'Save connection'}</Button></Dialog.Footer>
 </form>
</Dialog.Content></Dialog.Root>
{#if selected}<section class="mt-8 space-y-4"><h3 class="text-lg font-semibold">Operation history</h3>
 {#if registry}<p class="text-sm text-muted-foreground">Credential validation checks authentication for both accounts. It does not establish artifact push, pull or runtime readiness.</p>{/if}
 <Button variant="outline" disabled={busy||!!pendingKey} onclick={()=>perform(toggle)}>{selected.enabled?'Disable connection':'Enable connection'}</Button>
 {#if !registry&&selected.models?.length}<InformationCard title="Discovered models"><p class="text-sm text-muted-foreground">Choose a model when configuring a workspace. API access does not establish model compatibility with conversation execution.</p><ul class="max-h-64 overflow-y-auto font-mono text-xs">{#each selected.models as model (model)}<li>{model}</li>{/each}</ul></InformationCard>{/if}
 {#each history as op (op.id)}<InformationCard title={op.kind==='save'?'Credential candidate':'Connection state'} status={op.state}><InformationTable rows={[{label:'Operation',value:op.id,monospace:true},{label:'Revision',value:op.connectionRevision}]}/>{#if op.safeReason}<p class="text-sm">{reasonText[op.safeReason]||op.safeReason}</p>{/if}{#if op.kind==='save'&&op.state==='accepted'}<Button disabled={busy} onclick={()=>perform(async()=>{const result=await rpc('ValidateProviderConnection',{operationId:op.id});await accept(result.operation);})}>Validate candidate</Button>{/if}</InformationCard>{/each}
 {#if historyNext}<Button variant="outline" disabled={busy} onclick={()=>perform(()=>inspect(selected!,true))}>Load more history</Button>{/if}
</section>{/if}
